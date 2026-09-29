//! Private, cooperating multi-process storage for shared diagnostic records.

use crate::user_state::{self, ExistingLockState, FileLock, UserPaths};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use zterm_diagnostics::{Control, MAX_RECORD_BYTES, Sink, Snapshot};

/// Per-file desktop retention limit; each lane has a current file and one archive.
pub const FILE_BYTES: u64 = 4 * 1024 * 1024;
const LOCK_WAIT: Duration = Duration::from_secs(1);
const FILES: [(&str, bool); 4] = [
    ("daemon.log", false),
    ("daemon.log.1", false),
    ("daemon.debug.log", true),
    ("daemon.debug.log.1", true),
];

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Document {
    control: Control,
    // Binds new-writer capability to the exact daemon socket incarnation. Old
    // running binaries retain their permanent descriptors until normal replacement.
    daemon_socket: Option<[i64; 4]>,
}

/// Filesystem adapter; construction and inspection do not create state.
#[derive(Clone)]
pub struct Store {
    paths: UserPaths,
    limit: u64,
    registered_socket: Arc<Mutex<Option<[i64; 4]>>>,
}
impl Store {
    /// Uses account-authoritative paths and the desktop retention limits.
    pub fn new(paths: UserPaths) -> Self {
        Self {
            paths,
            limit: FILE_BYTES,
            registered_socket: Arc::default(),
        }
    }
    fn validate(&self) -> io::Result<()> {
        user_state::validate_directory(self.paths.state_root(), self.paths.uid())
            .map_err(io::Error::other)?;
        user_state::validate_directory(self.paths.logs(), self.paths.uid())
            .map_err(io::Error::other)
    }
    fn lock(&self, create: bool) -> io::Result<Option<FileLock>> {
        self.validate()?;
        let path = self.paths.logs().join("writer.lock");
        if !create && !exists(&path)? {
            return Ok(None);
        }
        let deadline = Instant::now() + LOCK_WAIT;
        loop {
            let lock = if create {
                FileLock::try_acquire(&path, self.paths.uid())
            } else {
                FileLock::try_acquire_existing(&path, self.paths.uid())
            }
            .map_err(io::Error::other)?;
            if lock.is_some() {
                return Ok(lock);
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "diagnostic writer busy",
                ));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn document(&self) -> io::Result<Document> {
        let path = self.paths.logs().join("diagnostics.json");
        if !exists(&path)? {
            return Ok(Document::default());
        }
        let file = user_state::open_read(&path, self.paths.uid()).map_err(io::Error::other)?;
        if file.metadata()?.len() > 1024 {
            return Ok(Document::default());
        }
        let mut bytes = Vec::new();
        file.take(1024).read_to_end(&mut bytes)?;
        Ok(serde_json::from_slice(&bytes).unwrap_or_default())
    }
    fn write_document(&self, document: &Document) -> io::Result<()> {
        let bytes = serde_json::to_vec(document)?;
        user_state::atomic_write(
            &self.paths.logs().join("diagnostics.json"),
            self.paths.uid(),
            |file| file.write_all(&bytes),
        )
        .map_err(io::Error::other)
    }
    /// Enables, renews or disables local detail without starting any daemon.
    pub fn set_control(&self, control: Control) -> io::Result<()> {
        let _lock = self.lock(true)?;
        let mut document = self.document()?;
        document.control = control;
        self.write_document(&document)
    }
    /// Marks the exact newly bound socket as using coordinated runtime rotation.
    /// The caller must already own the daemon lifetime lock.
    pub fn register_daemon(&self, _lock: &crate::local_unix::DaemonLock) -> io::Result<()> {
        let signature = self.socket_signature()?;
        // Keep the ownership proof before attempting IO: the worker retries
        // persistence after a competing writer or damaged control document.
        *self
            .registered_socket
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = signature;
        let _writer = self.lock(true)?;
        let mut document = self.document()?;
        document.daemon_socket = signature;
        self.write_document(&document)
    }
    // Called only with writer.lock held. Cached proof is valid solely for the
    // socket registered by this daemon, never for its replacement.
    fn repair_registration(&self) -> io::Result<()> {
        let signature = *self
            .registered_socket
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if signature.is_some() && signature == self.socket_signature()? {
            let mut document = self.document()?;
            if document.daemon_socket != signature {
                document.daemon_socket = signature;
                self.write_document(&document)?;
            }
        }
        Ok(())
    }
    fn socket_signature(&self) -> io::Result<Option<[i64; 4]>> {
        use std::os::unix::fs::MetadataExt;
        if !crate::local_unix::inspect_daemon_socket(&self.paths).map_err(io::Error::other)? {
            return Ok(None);
        }
        let metadata = fs::symlink_metadata(self.paths.socket())?;
        Ok(Some([
            metadata.dev() as i64,
            metadata.ino() as i64,
            metadata.ctime(),
            metadata.ctime_nsec(),
        ]))
    }
    /// A live old daemon prevents safe rotation until its normal replacement.
    pub fn retention_pending(&self) -> io::Result<bool> {
        let state = user_state::inspect_existing_lock(self.paths.daemon_lock(), self.paths.uid())
            .map_err(io::Error::other)?;
        if state != ExistingLockState::Locked {
            return Ok(false);
        }
        let signature = self.socket_signature()?;
        Ok(signature.is_none() || self.document()?.daemon_socket != signature)
    }
    /// Validates known log/control paths before an updater can stop the daemon.
    pub fn preflight(&self) -> io::Result<()> {
        let _lock = self.lock(true)?;
        for name in FILES
            .iter()
            .map(|(name, _)| *name)
            .chain(["diagnostics.json"])
        {
            let path = self.paths.logs().join(name);
            if exists(&path)? {
                user_state::validate_regular_file(&path, self.paths.uid())
                    .map_err(io::Error::other)?;
            }
        }
        // Probe actual append eligibility before irreversible handoff.
        user_state::open_append(self.paths.daemon_log(), self.paths.uid())
            .map_err(io::Error::other)?;
        Ok(())
    }
    fn normalize(&self, path: &Path) -> io::Result<()> {
        if !exists(path)? {
            return Ok(());
        }
        let mut file = user_state::open_read(path, self.paths.uid()).map_err(io::Error::other)?;
        let length = file.metadata()?.len();
        if length <= self.limit {
            return Ok(());
        }
        file.seek(SeekFrom::Start(length - self.limit))?;
        let mut tail = Vec::new();
        file.take(self.limit).read_to_end(&mut tail)?;
        let start = tail
            .iter()
            .position(|b| *b == b'\n')
            .map_or(tail.len(), |i| i + 1);
        let end = tail
            .iter()
            .rposition(|b| *b == b'\n')
            .map_or(start, |i| i + 1)
            .max(start);
        user_state::atomic_write(path, self.paths.uid(), |file| {
            file.write_all(&tail[start..end])
        })
        .map_err(io::Error::other)
    }
    /// Captures validated handles/lengths under short writer coordination.
    /// Reading/exporting happens after this function releases the lock.
    pub fn snapshots(&self, include_debug: bool) -> io::Result<Vec<Snapshot>> {
        if !exists(self.paths.logs())? {
            return Ok(Vec::new());
        }
        let _lock = self.lock(false)?;
        let mut snapshots = Vec::new();
        for (name, detail) in FILES {
            if detail && !include_debug {
                continue;
            }
            let path = self.paths.logs().join(name);
            if !exists(&path)? {
                continue;
            }
            let file = user_state::open_read(&path, self.paths.uid()).map_err(io::Error::other)?;
            let length = file.metadata()?.len();
            snapshots.push(Snapshot {
                file,
                length: length.min(self.limit),
                start: length.saturating_sub(self.limit),
                detail,
            });
        }
        Ok(snapshots)
    }
    /// Streams a private export, refusing overwrite and managed-state destinations.
    pub fn export(
        &self,
        output: &Path,
        include_debug: bool,
        pending_lost: u64,
    ) -> io::Result<zterm_diagnostics::ExportHeader> {
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let parent = parent.canonicalize()?;
        let state = self
            .paths
            .state_root()
            .canonicalize()
            .unwrap_or_else(|_| self.paths.state_root().to_path_buf());
        if parent.starts_with(state) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "export destination is managed state",
            ));
        }
        let filename = output.file_name().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "export destination needs a filename",
            )
        })?;
        let target = parent.join(filename);
        let mut snapshots = self.snapshots(include_debug)?;
        let mut file = user_state::create_new_file(&target).map_err(io::Error::other)?;
        let result =
            zterm_diagnostics::export(&mut snapshots, &mut file, include_debug, pending_lost)
                .and_then(|header| {
                    file.sync_all()?;
                    Ok(header)
                });
        if result.is_err() {
            let _ = fs::remove_file(target);
        }
        result
    }
}
impl Sink for Store {
    fn append(&self, detail: bool, bytes: &[u8]) -> io::Result<()> {
        if bytes.len() > MAX_RECORD_BYTES || !bytes.ends_with(b"\n") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid diagnostic record size",
            ));
        }
        let _lock = self.lock(true)?;
        self.repair_registration()?;
        let name = if detail {
            "daemon.debug.log"
        } else {
            "daemon.log"
        };
        let path = self.paths.logs().join(name);
        let archive = self.paths.logs().join(format!("{name}.1"));
        if !self.retention_pending()? {
            for (name, _) in FILES {
                self.normalize(&self.paths.logs().join(name))?;
            }
            let current =
                user_state::open_append(&path, self.paths.uid()).map_err(io::Error::other)?;
            if current.metadata()?.len() + bytes.len() as u64 > self.limit {
                drop(current);
                if exists(&archive)? {
                    user_state::validate_regular_file(&archive, self.paths.uid())
                        .map_err(io::Error::other)?;
                    fs::remove_file(&archive)?;
                }
                fs::rename(&path, &archive)?;
            }
        }
        let mut file =
            user_state::open_append(&path, self.paths.uid()).map_err(io::Error::other)?;
        file.write_all(bytes)
    }
    fn control(&self) -> io::Result<Control> {
        self.validate()?;
        let registered = *self
            .registered_socket
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if registered.is_some() && self.document()?.daemon_socket != registered {
            // The daemon may otherwise be idle while foreground/updater writers
            // keep appending. Repair on its worker poll as well as its writes.
            let _writer = self.lock(true)?;
            self.repair_registration()?;
        }
        Ok(self.document()?.control)
    }
    fn flush(&self) -> io::Result<()> {
        let _lock = self.lock(false)?;
        for (name, _) in FILES {
            let path = self.paths.logs().join(name);
            if exists(&path)? {
                user_state::open_read(&path, self.paths.uid())
                    .map_err(io::Error::other)?
                    .sync_data()?;
            }
        }
        fs::File::open(self.paths.logs())?.sync_all()
    }
}
fn exists(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use zterm_diagnostics::{Event, Filter, Kind, Recorder};

    fn fixture(limit: u64) -> (tempfile::TempDir, Store) {
        let temp = tempfile::tempdir().expect("private fixture");
        let paths = UserPaths::for_test(
            nix::unistd::geteuid().as_raw(),
            temp.path().into(),
            temp.path().join("state"),
            temp.path().join("runtime"),
        );
        paths.prepare_state_directories().expect("private paths");
        (
            temp,
            Store {
                limit,
                ..Store::new(paths)
            },
        )
    }
    #[test]
    fn concurrent_rotation_separates_lanes_and_readers_keep_snapshot_handles() {
        let (_temp, store) = fixture(8192);
        store
            .set_control(Control::enabled(zterm_diagnostics::now_ms()))
            .expect("enable detail");
        // Independent adapters acquire the same stable inode, as independent processes do.
        let workers: Vec<_> = (0..4)
            .map(|_| {
                let store = store.clone();
                std::thread::spawn(move || {
                    let recorder = Recorder::new(Arc::new(store)).expect("recorder");
                    for _ in 0..50 {
                        recorder.record(Event::new(Kind::ConnectionStarted));
                    }
                    assert!(recorder.flush(Duration::from_secs(5)));
                })
            })
            .collect();
        for worker in workers {
            worker.join().expect("writer");
        }
        let mut snapshots = store.snapshots(false).expect("snapshot");
        let old = zterm_diagnostics::tail(
            &mut snapshots,
            &Filter {
                lines: 1000,
                json: true,
                ..Default::default()
            },
        )
        .expect("old tail");
        assert!(!old.lines.is_empty());
        let key_files: Vec<_> = FILES[..2]
            .iter()
            .map(|(name, _)| fs::read(store.paths.logs().join(name)).expect("key file"))
            .collect();
        let recorder = Recorder::new(Arc::new(store.clone())).expect("recorder");
        let deadline = Instant::now() + Duration::from_secs(2);
        while !recorder.detail_enabled() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(recorder.flush(Duration::from_secs(2)));
        // Capture after the interval transition key event, then write detail only.
        let key = fs::read(store.paths.daemon_log()).expect("key");
        for _ in 0..100 {
            store
                .append(true, b"detail lane fixture\n")
                .expect("detail append");
        }
        assert!(recorder.flush(Duration::from_secs(5)));
        assert_eq!(key, fs::read(store.paths.daemon_log()).expect("key"));
        assert!(
            key_files
                .iter()
                .all(|bytes| bytes.len() <= store.limit as usize)
        );
        for (name, _) in FILES {
            let path = store.paths.logs().join(name);
            if path.exists() {
                assert!(fs::metadata(path).expect("size").len() <= store.limit);
            }
        }
        let again = zterm_diagnostics::tail(
            &mut snapshots,
            &Filter {
                lines: 1000,
                json: true,
                ..Default::default()
            },
        )
        .expect("snapshot tail");
        assert_eq!(old.lines, again.lines);
    }
    #[test]
    fn safe_paths_control_export_legacy_and_zero_state_inspection() {
        let (temp, store) = fixture(8192);
        assert!(store.snapshots(false).expect("empty").is_empty());
        assert!(!store.paths.logs().join("writer.lock").exists());
        assert_eq!(store.control().expect("off"), Control::default());
        let outside = temp.path().join("outside");
        fs::write(&outside, "unchanged").expect("sentinel");
        std::os::unix::fs::symlink(&outside, store.paths.daemon_log()).expect("unsafe link");
        assert!(store.append(false, b"safe\n").is_err());
        assert_eq!(fs::read_to_string(&outside).expect("sentinel"), "unchanged");
        fs::remove_file(store.paths.daemon_log()).expect("link cleanup");
        store
            .append(false, b"legacy event\n")
            .expect("legacy fixture");
        let recorder = Recorder::new(Arc::new(store.clone())).expect("recorder");
        recorder.record(Event::new(Kind::UploadCompleted));
        assert!(recorder.flush(Duration::from_secs(2)));
        let output = temp.path().join("export.jsonl");
        let header = store.export(&output, false, 0).expect("export");
        assert_eq!(header.omitted, 1);
        let content = fs::read_to_string(&output).expect("export contents");
        assert!(!content.contains("legacy event"));
        assert!(content.contains("upload_completed"));
        assert!(store.export(&output, false, 0).is_err());
        assert!(
            store
                .export(&store.paths.logs().join("export.jsonl"), false, 0)
                .is_err()
        );
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(output)
                .expect("private export")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    #[test]
    fn oversized_legacy_tail_is_normalized_and_partial_lines_are_not_imported() {
        let (_temp, store) = fixture(8192);
        let mut file =
            user_state::open_append(store.paths.daemon_log(), store.paths.uid()).expect("legacy");
        file.write_all(&vec![b'x'; 20_000]).expect("oversize");
        file.write_all(b"\nlast legacy event\n")
            .expect("legacy tail");
        drop(file);
        let recorder = Recorder::new(Arc::new(store.clone())).expect("recorder");
        recorder.record(Event::new(Kind::DaemonReady));
        assert!(recorder.flush(Duration::from_secs(2)));
        let current = fs::read_to_string(store.paths.daemon_log()).expect("normalized");
        assert!(current.starts_with("last legacy event\n"));
        assert!(current.len() < 8192);
        let mut snapshots = store.snapshots(false).expect("snapshots");
        let tail = zterm_diagnostics::tail(
            &mut snapshots,
            &Filter {
                lines: 100,
                json: true,
                ..Default::default()
            },
        )
        .expect("tail");
        assert_eq!(tail.omitted, 1);
        assert_eq!(tail.lines.len(), 1);
    }
}

#[cfg(test)]
mod process_tests {
    use super::*;
    use std::sync::Arc;
    use zterm_diagnostics::{Event, Kind, Recorder};

    fn paths(root: &Path) -> UserPaths {
        UserPaths::for_test(
            nix::unistd::geteuid().as_raw(),
            root.into(),
            root.join("state"),
            root.join("run"),
        )
    }
    #[test]
    fn cooperating_processes_rotate_complete_records_on_one_stable_lock() {
        let temp = tempfile::tempdir().expect("private root");
        let paths = paths(temp.path());
        paths.prepare_state_directories().expect("paths");
        let store = Store {
            limit: 8192,
            ..Store::new(paths.clone())
        };
        store.preflight().expect("writer preflight");
        use std::os::unix::fs::MetadataExt;
        let lock_inode = fs::metadata(paths.logs().join("writer.lock"))
            .expect("stable lock")
            .ino();
        let children: Vec<_> = (0..3)
            .map(|_| {
                std::process::Command::new(std::env::current_exe().expect("test executable"))
                    .args([
                        "--exact",
                        "diagnostics::process_tests::writer_child",
                        "--ignored",
                    ])
                    .env("ZTERM_DIAGNOSTICS_TEST_ROOT", temp.path())
                    .stdout(std::process::Stdio::piped())
                    .spawn()
                    .expect("writer child")
            })
            .collect();
        let outcomes: Vec<_> = children
            .into_iter()
            .map(|child| child.wait_with_output().expect("writer exit"))
            .collect();
        for outcome in outcomes {
            assert!(
                outcome.status.success(),
                "{}",
                String::from_utf8_lossy(&outcome.stdout)
            );
        }
        assert_eq!(
            fs::metadata(paths.logs().join("writer.lock"))
                .expect("same lock")
                .ino(),
            lock_inode
        );
        let mut records = 0;
        for snapshot in store.snapshots(false).expect("snapshots") {
            assert!(snapshot.length <= 8192);
            let mut text = String::new();
            snapshot
                .file
                .take(snapshot.length)
                .read_to_string(&mut text)
                .expect("bounded read");
            for line in text.lines() {
                assert!(zterm_diagnostics::Record::decode(line.as_bytes()).is_some());
                records += 1;
            }
        }
        assert!(records > 1);
    }
    #[test]
    #[ignore = "isolated writer fixture"]
    fn writer_child() {
        let root = std::env::var_os("ZTERM_DIAGNOSTICS_TEST_ROOT").expect("test-private root");
        let store = Store {
            limit: 8192,
            ..Store::new(paths(Path::new(&root)))
        };
        let recorder = Recorder::new(Arc::new(store)).expect("writer");
        for _ in 0..40 {
            while !recorder.record(Event::new(Kind::ConnectionStarted)) {
                std::thread::yield_now();
            }
            assert!(recorder.flush(Duration::from_secs(3)));
        }
    }
    #[test]
    fn old_daemon_descriptor_is_preserved_until_exact_new_socket_registration() {
        let temp = tempfile::tempdir().expect("private root");
        let paths = paths(temp.path());
        paths.prepare_state_directories().expect("paths");
        let daemon = crate::local_unix::DaemonLock::try_acquire(&paths)
            .expect("daemon lock")
            .expect("exclusive");
        let (_listener, _ownership) =
            crate::local_unix::bind_owned_daemon_socket(&paths, &daemon).expect("old socket");
        let store = Store {
            limit: 8192,
            ..Store::new(paths.clone())
        };
        let old_descriptor =
            user_state::open_append(paths.daemon_log(), paths.uid()).expect("old writer");
        assert!(store.retention_pending().expect("compatibility"));
        for _ in 0..300 {
            store
                .append(false, b"legacy-compatible record with complete line\n")
                .expect("compatibility append");
        }
        assert!(old_descriptor.metadata().expect("old inode").len() > 8192);
        assert!(!paths.logs().join("daemon.log.1").exists());
        store
            .register_daemon(&daemon)
            .expect("new writer registration");
        assert!(!store.retention_pending().expect("participating daemon"));
        store
            .append(false, b"new writer\n")
            .expect("bounded cutover");
        assert!(fs::metadata(paths.daemon_log()).expect("current").len() <= 8192);
        for snapshot in store.snapshots(false).expect("bounded retention") {
            assert!(snapshot.file.metadata().expect("retained file").len() <= 8192);
        }
    }

    #[test]
    fn registered_daemon_recovers_retention_after_control_loss_and_registration_contention() {
        let temp = tempfile::tempdir().expect("private root");
        let paths = paths(temp.path());
        paths.prepare_state_directories().expect("paths");
        let daemon = crate::local_unix::DaemonLock::try_acquire(&paths)
            .expect("daemon lock")
            .expect("exclusive");
        let (_listener, _ownership) =
            crate::local_unix::bind_owned_daemon_socket(&paths, &daemon).expect("socket");
        let store = Store {
            limit: 8192,
            ..Store::new(paths.clone())
        };
        store.register_daemon(&daemon).expect("registration");
        let worker = store.clone();
        for damage in [None, Some(b"broken".as_slice()), Some(&[b'x'; 1025])] {
            let control = paths.logs().join("diagnostics.json");
            fs::remove_file(&control).expect("remove control");
            if let Some(bytes) = damage {
                user_state::atomic_write(&control, paths.uid(), |file| file.write_all(bytes))
                    .expect("damaged control");
            }
            assert_eq!(worker.control().expect("detail off"), Control::default());
            assert!(
                !Store::new(paths.clone())
                    .retention_pending()
                    .expect("idle daemon poll repairs independent writers")
            );
            for _ in 0..300 {
                worker
                    .append(false, b"complete registered daemon record\n")
                    .expect("bounded append");
            }
            for snapshot in worker.snapshots(false).expect("retained files") {
                assert!(snapshot.file.metadata().expect("size").len() <= 8192);
            }
            assert!(
                !Store::new(paths.clone())
                    .retention_pending()
                    .expect("repaired marker")
            );
        }

        // Registration must survive a transient competing writer before it can
        // persist the marker. The recorder uses a clone of this exact adapter.
        fs::remove_file(paths.logs().join("diagnostics.json")).expect("remove marker");
        let writer = store.lock(true).expect("competing writer");
        let contended = Store::new(paths.clone());
        let worker = contended.clone();
        assert!(contended.register_daemon(&daemon).is_err());
        drop(writer);
        worker
            .append(false, b"after contention\n")
            .expect("recovery");
        assert!(
            !Store::new(paths.clone())
                .retention_pending()
                .expect("repaired registration")
        );

        fs::rename(paths.socket(), temp.path().join("prior.sock")).expect("old incarnation");
        let (_replacement, _ownership) =
            crate::local_unix::bind_owned_daemon_socket(&paths, &daemon).expect("replacement");
        fs::remove_file(paths.logs().join("diagnostics.json")).expect("unregistered replacement");
        worker
            .append(false, b"legacy replacement\n")
            .expect("compatible append");
        assert!(
            Store::new(paths)
                .retention_pending()
                .expect("cached proof must not transfer")
        );
    }
}
