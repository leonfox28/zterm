use std::{
    fs::File,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
    time::Instant,
};
use tokio::{
    sync::{Semaphore, watch},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
use zterm_client::{
    error::ClientError,
    upload::{UploadConnector, UploadOrigin},
};
use zterm_core::{
    DomainErrorKind,
    upload::{MAX_UPLOAD_BYTES, UploadMetadata, UploadPhase, UploadProgress, UploadedFile},
};

static CLIPBOARD_JOB: OnceLock<Arc<Semaphore>> = OnceLock::new();

pub(super) struct DesktopUpload {
    pub(super) epoch: u64,
    pub(super) origin: UploadOrigin,
    pub(super) progress: watch::Receiver<UploadProgress>,
    pub(super) task: JoinHandle<Result<UploadedFile, ClientError>>,
    cancel: CancellationToken,
}

impl DesktopUpload {
    pub(super) fn start(
        connector: Arc<dyn UploadConnector>,
        origin: UploadOrigin,
        epoch: u64,
    ) -> Self {
        let cancel = CancellationToken::new();
        let cancellation = cancel.clone();
        let (updates, progress) = watch::channel(zterm_client::upload::preparing());
        let task = tokio::spawn(async move {
            let semaphore = CLIPBOARD_JOB.get_or_init(|| Arc::new(Semaphore::new(1)));
            let permit = semaphore
                .clone()
                .try_acquire_owned()
                .map_err(|_| local_error("Clipboard is still busy; try again shortly"))?;
            let source = tokio::select! {
                biased;
                _ = cancellation.cancelled() => return Err(cancelled()),
                result = tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    let mut clipboard = arboard::Clipboard::new().map_err(|_| local_error("System clipboard is unavailable"))?;
                    prepare_clipboard(&mut clipboard)
                }) => result.map_err(|_| local_error("Could not read clipboard"))??,
            };
            let PreparedSource {
                file,
                metadata,
                _temporary,
            } = source;
            zterm_client::upload::upload(
                &*connector,
                origin.target,
                origin.binding,
                metadata,
                tokio::fs::File::from_std(file),
                &updates,
                &cancellation,
            )
            .await
        });
        Self {
            epoch,
            origin,
            progress,
            task,
            cancel,
        }
    }

    pub(super) fn cancel(&self) {
        self.cancel.cancel();
    }
    pub(super) fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
}
impl Drop for DesktopUpload {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.task.abort();
    }
}

trait ClipboardSource {
    fn files(&mut self) -> Result<Vec<PathBuf>, arboard::Error>;
    fn image(&mut self) -> Result<arboard::ImageData<'static>, arboard::Error>;
}
impl ClipboardSource for arboard::Clipboard {
    fn files(&mut self) -> Result<Vec<PathBuf>, arboard::Error> {
        self.get().file_list()
    }
    fn image(&mut self) -> Result<arboard::ImageData<'static>, arboard::Error> {
        self.get().image()
    }
}
struct PreparedSource {
    file: File,
    metadata: UploadMetadata,
    _temporary: Option<tempfile::TempPath>,
}

fn prepare_clipboard(clipboard: &mut impl ClipboardSource) -> Result<PreparedSource, ClientError> {
    match clipboard.files() {
        Ok(files) if files.len() == 1 => return open_file(&files[0]),
        Ok(files) if !files.is_empty() => {
            return Err(local_error("Copy exactly one file to upload"));
        }
        Ok(_) | Err(arboard::Error::ContentNotAvailable) => {}
        Err(_) => return Err(local_error("Could not read clipboard files")),
    }
    let image = clipboard.image().map_err(|_| {
        local_error("Copy one file or an image first; plain text paths are not uploaded")
    })?;
    encode_image(image)
}

fn open_file(path: &Path) -> Result<PreparedSource, ClientError> {
    // Inspect before open so a copied FIFO cannot block a clipboard worker.
    if !std::fs::metadata(path)
        .map_err(|_| local_error("Copied file is unavailable"))?
        .is_file()
    {
        return Err(local_error(
            "Choose a regular file; folders cannot be uploaded",
        ));
    }
    use std::os::unix::fs::OpenOptionsExt;
    // Also fence replacement by a FIFO between metadata and open. Regular file
    // reads are unaffected by O_NONBLOCK; the opened descriptor is checked below.
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| local_error("Copied file cannot be opened"))?;
    let info = file
        .metadata()
        .map_err(|_| local_error("Copied file cannot be inspected"))?;
    if !info.is_file() {
        return Err(local_error("Choose a regular file"));
    }
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("");
    let metadata = UploadMetadata::new(info.len(), extension)
        .map_err(|kind| ClientError::new(kind, "File exceeds 50 MB"))?;
    Ok(PreparedSource {
        file,
        metadata,
        _temporary: None,
    })
}

fn encode_image(image: arboard::ImageData<'_>) -> Result<PreparedSource, ClientError> {
    let length = image
        .width
        .checked_mul(image.height)
        .and_then(|length| length.checked_mul(4));
    let width = u32::try_from(image.width)
        .map_err(|_| local_error("Clipboard image dimensions are invalid"))?;
    let height = u32::try_from(image.height)
        .map_err(|_| local_error("Clipboard image dimensions are invalid"))?;
    if width == 0 || height == 0 || length != Some(image.bytes.len()) {
        return Err(local_error("Clipboard image dimensions are invalid"));
    }
    let temporary = tempfile::NamedTempFile::new()
        .map_err(|_| local_error("Could not prepare clipboard image"))?;
    let mut sink = BoundedPng {
        file: temporary.as_file(),
        written: 0,
        oversized: false,
    };
    let encoded = (|| {
        let mut encoder = png::Encoder::new(&mut sink, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&image.bytes)?;
        writer.finish()
    })();
    if sink.oversized {
        return Err(ClientError::new(
            DomainErrorKind::UploadTooLarge,
            "Clipboard PNG exceeds 50 MB",
        ));
    }
    encoded.map_err(|_| local_error("Could not encode clipboard image"))?;
    let metadata = UploadMetadata::new(sink.written, "png")
        .map_err(|kind| ClientError::new(kind, "Clipboard PNG exceeds 50 MB"))?;
    let file = temporary
        .reopen()
        .map_err(|_| local_error("Could not open prepared image"))?;
    Ok(PreparedSource {
        file,
        metadata,
        _temporary: Some(temporary.into_temp_path()),
    })
}

struct BoundedPng<'a> {
    file: &'a File,
    written: u64,
    oversized: bool,
}
impl Write for BoundedPng<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.written.saturating_add(bytes.len() as u64) > MAX_UPLOAD_BYTES {
            self.oversized = true;
            return Err(io::Error::other("PNG exceeds upload limit"));
        }
        let length = self.file.write(bytes)?;
        self.written += length as u64;
        Ok(length)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

fn local_error(message: &'static str) -> ClientError {
    ClientError::new(DomainErrorKind::UploadSourceInvalid, message)
}
fn cancelled() -> ClientError {
    ClientError::new(DomainErrorKind::Cancelled, "Upload cancelled")
}

pub(super) struct UploadStatus {
    progress: UploadProgress,
    sampled: Instant,
    sampled_bytes: u64,
    speed: f64,
    notice: Option<String>,
    expires: Option<Instant>,
}
impl UploadStatus {
    pub(super) fn preparing() -> Self {
        Self {
            progress: zterm_client::upload::preparing(),
            sampled: Instant::now(),
            sampled_bytes: 0,
            speed: 0.0,
            notice: None,
            expires: None,
        }
    }
    pub(super) fn notice(text: String, temporary: bool) -> Self {
        Self {
            notice: Some(text),
            expires: temporary.then(|| Instant::now() + std::time::Duration::from_secs(3)),
            ..Self::preparing()
        }
    }
    pub(super) fn observe(&mut self, progress: UploadProgress) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.sampled).as_secs_f64();
        if elapsed >= 0.25 {
            let speed = progress.accepted_bytes.saturating_sub(self.sampled_bytes) as f64 / elapsed;
            self.speed = if self.sampled_bytes == 0 {
                speed
            } else {
                self.speed * 0.65 + speed * 0.35
            };
            self.sampled = now;
            self.sampled_bytes = progress.accepted_bytes;
        }
        self.progress = progress;
    }
    pub(super) fn expired(&self) -> bool {
        self.expires
            .is_some_and(|deadline| Instant::now() >= deadline)
    }
    pub(super) fn is_error(&self) -> bool {
        self.notice.is_some() && self.expires.is_none()
    }
    pub(super) fn text(&self, width: usize) -> String {
        if let Some(notice) = &self.notice {
            return truncate(notice, width);
        }
        if self.progress.phase == UploadPhase::Preparing {
            return truncate("Preparing upload…  Ctrl+] c: cancel", width);
        }
        let size = self.progress.total_bytes.unwrap_or(0);
        let percent = (self.progress.accepted_bytes * 100)
            .checked_div(size)
            .unwrap_or(100);
        let percent = format!("{percent}%");
        let filled = (self.progress.accepted_bytes * 10)
            .checked_div(size)
            .unwrap_or(10) as usize;
        let bar = format!(
            "[{}{}] {percent}",
            "=".repeat(filled),
            " ".repeat(10 - filled)
        );
        let full = format!(
            "{} {}/{} MB {:.1} MB/s",
            if self.progress.phase == UploadPhase::Finishing {
                "Saving… 100%"
            } else {
                &bar
            },
            format_args!("{:.1}", self.progress.accepted_bytes as f64 / 1_000_000.0),
            format_args!("{:.1}", size as f64 / 1_000_000.0),
            self.speed / 1_000_000.0
        );
        [full, bar, percent]
            .into_iter()
            .find(|text| unicode_width::UnicodeWidthStr::width(text.as_str()) <= width)
            .unwrap_or_else(|| truncate("%", width))
    }
}
pub(super) fn truncate(text: &str, width: usize) -> String {
    let mut remaining = width;
    text.chars()
        .take_while(|character| {
            let cells = unicode_width::UnicodeWidthChar::width(*character).unwrap_or(0);
            if cells > remaining {
                false
            } else {
                remaining -= cells;
                true
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    struct Clipboard {
        files: Vec<PathBuf>,
        image: bool,
        image_reads: usize,
    }
    impl ClipboardSource for Clipboard {
        fn files(&mut self) -> Result<Vec<PathBuf>, arboard::Error> {
            Ok(self.files.clone())
        }
        fn image(&mut self) -> Result<arboard::ImageData<'static>, arboard::Error> {
            self.image_reads += 1;
            if self.image {
                Ok(arboard::ImageData {
                    width: 2,
                    height: 1,
                    bytes: vec![255, 0, 0, 255, 0, 255, 0, 255].into(),
                })
            } else {
                Err(arboard::Error::ContentNotAvailable)
            }
        }
    }

    #[test]
    fn explicit_single_file_wins_and_ambiguous_or_directory_sources_never_fall_back() {
        let tmp = tempfile::tempdir().expect("source fixture");
        let path = tmp.path().join("arbitrary.PDF");
        std::fs::write(&path, b"unchanged original bytes").expect("source fixture");
        let mut clipboard = Clipboard {
            files: vec![path.clone()],
            image: true,
            image_reads: 0,
        };
        let mut prepared = prepare_clipboard(&mut clipboard).expect("single file");
        let mut bytes = Vec::new();
        prepared
            .file
            .read_to_end(&mut bytes)
            .expect("read original");
        assert_eq!(bytes, b"unchanged original bytes");
        assert_eq!(prepared.metadata.extension(), "pdf");
        assert_eq!(clipboard.image_reads, 0);
        clipboard.files.push(path);
        assert!(prepare_clipboard(&mut clipboard).is_err());
        clipboard.files = vec![tmp.path().to_path_buf()];
        assert!(prepare_clipboard(&mut clipboard).is_err());
        assert_eq!(clipboard.image_reads, 0);
    }

    #[test]
    fn raw_images_become_exact_png_pixels_and_plain_text_has_no_file_fallback() {
        let mut clipboard = Clipboard {
            files: Vec::new(),
            image: true,
            image_reads: 0,
        };
        let source = prepare_clipboard(&mut clipboard).expect("raw image");
        assert_eq!(source.metadata.extension(), "png");
        assert_eq!(
            source.file.metadata().expect("PNG metadata").len(),
            source.metadata.size()
        );
        let mut decoder = png::Decoder::new(std::io::BufReader::new(source.file))
            .read_info()
            .expect("decode staged PNG");
        let mut pixels = vec![0; decoder.output_buffer_size().expect("bounded image")];
        decoder.next_frame(&mut pixels).expect("decode pixels");
        assert_eq!(pixels, [255, 0, 0, 255, 0, 255, 0, 255]);
        clipboard.image = false;
        assert!(prepare_clipboard(&mut clipboard).is_err());
    }

    #[test]
    fn copied_file_size_limit_is_inclusive_and_type_independent() {
        let tmp = tempfile::NamedTempFile::new().expect("size fixture");
        tmp.as_file()
            .set_len(MAX_UPLOAD_BYTES)
            .expect("sparse boundary file");
        assert!(open_file(tmp.path()).is_ok());
        tmp.as_file()
            .set_len(MAX_UPLOAD_BYTES + 1)
            .expect("oversized boundary");
        assert!(
            matches!(open_file(tmp.path()), Err(error) if error.kind() == DomainErrorKind::UploadTooLarge)
        );
        tmp.as_file().set_len(0).expect("empty file");
        assert_eq!(
            open_file(tmp.path())
                .expect("empty file accepted")
                .metadata
                .size(),
            0
        );
    }

    #[test]
    fn narrow_status_retains_percentage_and_never_splits_wide_cells() {
        let mut status = UploadStatus::preparing();
        status.observe(UploadProgress {
            phase: UploadPhase::Uploading,
            accepted_bytes: 25_000_000,
            total_bytes: Some(MAX_UPLOAD_BYTES),
        });
        assert_eq!(status.text(3), "50%");
        assert!(status.text(80).contains("25.0/50.0 MB"));
        for width in 1..80 {
            assert!(unicode_width::UnicodeWidthStr::width(status.text(width).as_str()) <= width);
        }
        assert_eq!(truncate("你好a", 3), "你");
        assert_eq!(truncate("你好a", 4), "你好");
    }
}
