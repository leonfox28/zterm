//! Content-independent contracts for one explicit remote file upload.

use std::{fmt, time::Duration};

pub use crate::domain::TransferId;
use crate::{AttachmentId, DomainErrorKind, SessionId};

/// Inclusive decimal 50 MB limit, shared by sources, wire admission and storage.
pub const MAX_UPLOAD_BYTES: u64 = 50_000_000;
/// Largest file-data chunk in a single upload frame.
pub const UPLOAD_CHUNK_BYTES: usize = 65_536;
/// Maximum sent bytes awaiting host acceptance.
pub const UPLOAD_WINDOW_BYTES: u64 = 1_048_576;
/// Host acknowledgement byte interval.
pub const UPLOAD_ACK_BYTES: u64 = 262_144;
/// Maximum delay between progress observations while bytes are accepted.
pub const UPLOAD_PROGRESS_INTERVAL: Duration = Duration::from_millis(250);
/// Maximum inactivity during streaming; independent of the control deadline.
pub const UPLOAD_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// Largest useful ASCII filename extension retained in generated remote paths.
pub const MAX_UPLOAD_EXTENSION_BYTES: usize = 16;
/// Maximum safe absolute path returned by the host.
pub const MAX_UPLOAD_PATH_BYTES: usize = 1024;

/// Exact live controller attachment to which an upload belongs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UploadBinding {
    /// Original session, never re-resolved by name.
    pub session_id: SessionId,
    /// Original attachment, never replaced following reconnect.
    pub attachment_id: AttachmentId,
}

/// Validated size and optional harmless extension; original names stay local.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UploadMetadata {
    size: u64,
    extension: String,
}

impl UploadMetadata {
    /// Validates the byte limit and normalizes an optional source extension.
    pub fn new(size: u64, extension: &str) -> Result<Self, DomainErrorKind> {
        if size > MAX_UPLOAD_BYTES {
            return Err(DomainErrorKind::UploadTooLarge);
        }
        let extension = if extension.len() <= MAX_UPLOAD_EXTENSION_BYTES
            && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
        {
            extension.to_ascii_lowercase()
        } else {
            String::new()
        };
        Ok(Self { size, extension })
    }

    /// Exact declared source size, including zero.
    #[must_use]
    pub const fn size(&self) -> u64 {
        self.size
    }

    /// Safe ASCII extension without a dot, or empty.
    #[must_use]
    pub fn extension(&self) -> &str {
        &self.extension
    }
}

/// Upload phase exposed to both frontends.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UploadPhase {
    /// Acquiring or staging a source, before host byte acceptance.
    Preparing,
    /// Host is accepting chunks.
    Uploading,
    /// All bytes accepted; waiting for publication.
    Finishing,
    /// Host published the complete file.
    Completed,
}

/// Latest-only upload observation. Accepted bytes are acknowledged host writes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UploadProgress {
    /// Current phase.
    pub phase: UploadPhase,
    /// Bytes accepted by the host.
    pub accepted_bytes: u64,
    /// Declared total size, absent while preparing an unknown source.
    pub total_bytes: Option<u64>,
}

/// Successful publication, validated before any terminal paste.
#[derive(Clone, Eq, PartialEq)]
pub struct UploadedFile {
    path: String,
    size: u64,
}

impl UploadedFile {
    /// Requires a safe generated absolute path with no shell or paste controls.
    pub fn new(path: String, size: u64) -> Result<Self, DomainErrorKind> {
        if size > MAX_UPLOAD_BYTES
            || path.len() > MAX_UPLOAD_PATH_BYTES
            || !path.starts_with("/tmp/zterm-")
            || path
                .split('/')
                .skip(1)
                .any(|part| part.is_empty() || part == "." || part == "..")
            || !path
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"/._-".contains(&byte))
        {
            return Err(DomainErrorKind::MalformedFrame);
        }
        Ok(Self { path, size })
    }

    /// Absolute safe path for mode-aware paste, without an Enter key.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Exact published size.
    #[must_use]
    pub const fn size(&self) -> u64 {
        self.size
    }
}

impl fmt::Debug for UploadedFile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UploadedFile")
            .field("path", &"[REDACTED]")
            .field("size", &self.size)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admission_preserves_empty_files_and_decimal_limit() {
        assert_eq!(
            UploadMetadata::new(0, "PDF")
                .expect("upload fixture")
                .extension(),
            "pdf"
        );
        assert!(UploadMetadata::new(MAX_UPLOAD_BYTES, "").is_ok());
        assert_eq!(
            UploadMetadata::new(MAX_UPLOAD_BYTES + 1, "png"),
            Err(DomainErrorKind::UploadTooLarge)
        );
        assert_eq!(
            UploadMetadata::new(1, "../pdf")
                .expect("upload fixture")
                .extension(),
            ""
        );
    }

    #[test]
    fn returned_paths_cannot_escape_or_inject_terminal_input() {
        for path in [
            "/tmp/zterm-1/../file",
            "/tmp/zterm-1/file\n",
            "/tmp/zterm-1/$(id)",
            "/etc/passwd",
            "/tmp/zterm-1//file",
        ] {
            assert!(UploadedFile::new(path.into(), 1).is_err());
        }
        assert!(UploadedFile::new("/tmp/zterm-1/abcd/ef01/file.pdf".into(), 0).is_ok());
    }
}
