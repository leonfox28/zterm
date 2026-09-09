//! Private effective-user upload staging. Only explicit successful publication persists.

use crate::user_state::{create_new_file, create_or_validate_directory, validate_directory};
use std::{
    fs::{self, File},
    io::{self, Write},
    os::unix::fs::DirBuilderExt,
    path::PathBuf,
};
use zterm_core::{
    SessionId,
    upload::{TransferId, UploadMetadata, UploadedFile},
};

/// One unique staging directory. Drop removes only an incomplete attempt.
pub struct StagedUpload {
    file: File,
    directory: PathBuf,
    public_path: String,
    metadata: UploadMetadata,
    accepted: u64,
    committed: bool,
}

impl StagedUpload {
    /// Creates private directories beneath the system `/tmp` alias, not `$TMPDIR`.
    pub fn create(
        session: SessionId,
        id: TransferId,
        metadata: UploadMetadata,
    ) -> io::Result<Self> {
        let uid = nix::unistd::geteuid().as_raw();
        let name = format!("zterm-{uid}");
        let root = fs::canonicalize("/tmp")?.join(&name);
        Self::create_at(root, format!("/tmp/{name}"), uid, session, id, metadata)
    }

    fn create_at(
        root: PathBuf,
        public_root: String,
        uid: u32,
        session: SessionId,
        id: TransferId,
        metadata: UploadMetadata,
    ) -> io::Result<Self> {
        create_or_validate_directory(&root, uid).map_err(private_path_error)?;
        let session_name = session.to_string();
        let parent = root.join(&session_name);
        create_or_validate_directory(&parent, uid).map_err(private_path_error)?;
        let transfer_name: String = id
            .as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let directory = parent.join(&transfer_name);
        // A collision never reuses or removes another transfer's directory.
        fs::DirBuilder::new().mode(0o700).create(&directory)?;
        let created = (|| {
            validate_directory(&directory, uid).map_err(private_path_error)?;
            create_new_file(&directory.join(".part")).map_err(private_path_error)
        })();
        let file = match created {
            Ok(file) => file,
            Err(error) => {
                let _ = fs::remove_dir(&directory);
                return Err(error);
            }
        };
        let filename = if metadata.extension().is_empty() {
            "file".into()
        } else {
            format!("file.{}", metadata.extension())
        };
        Ok(Self {
            file,
            directory,
            public_path: format!("{public_root}/{session_name}/{transfer_name}/{filename}"),
            metadata,
            accepted: 0,
            committed: false,
        })
    }

    /// Writes only exact sequential bytes within the declared size.
    pub fn write_chunk(&mut self, offset: u64, data: &[u8]) -> io::Result<()> {
        let end = offset
            .checked_add(data.len() as u64)
            .ok_or_else(invalid_bytes)?;
        if offset != self.accepted || end > self.metadata.size() || data.is_empty() {
            return Err(invalid_bytes());
        }
        self.file.write_all(data)?;
        self.accepted = end;
        Ok(())
    }

    /// Accepted file bytes, independently counted from incoming metadata.
    #[must_use]
    pub const fn accepted_bytes(&self) -> u64 {
        self.accepted
    }

    /// Syncs and publishes one complete file. Drop never removes it afterwards.
    pub fn publish(mut self) -> io::Result<UploadedFile> {
        if self.accepted != self.metadata.size() {
            return Err(invalid_bytes());
        }
        let result = UploadedFile::new(self.public_path.clone(), self.accepted)
            .map_err(|_| invalid_bytes())?;
        self.file.sync_all()?;
        let filename = self
            .public_path
            .rsplit('/')
            .next()
            .ok_or_else(invalid_bytes)?;
        fs::rename(self.directory.join(".part"), self.directory.join(filename))?;
        // From this point, even a directory-sync failure has an uncertain committed outcome.
        self.committed = true;
        File::open(&self.directory)?.sync_all()?;
        Ok(result)
    }
}

impl Drop for StagedUpload {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(self.directory.join(".part"));
            let _ = fs::remove_dir(&self.directory);
        }
    }
}

fn invalid_bytes() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "upload byte count mismatch")
}
fn private_path_error(_: crate::user_state::PathError) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "upload directory failed ownership, type or mode validation",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt, symlink};

    fn create(root: PathBuf, id: u8, size: u64) -> io::Result<StagedUpload> {
        StagedUpload::create_at(
            root,
            "/tmp/zterm-1".into(),
            nix::unistd::geteuid().as_raw(),
            SessionId::from_array([1; 16]),
            TransferId::from_array([id; 16]),
            UploadMetadata::new(size, "pdf").expect("upload fixture"),
        )
    }

    #[test]
    fn publication_keeps_exact_private_bytes_and_cancel_removes_only_staging() {
        let tmp = tempfile::tempdir().expect("upload fixture");
        let root = tmp.path().join("uploads");
        let mut staged = create(root.clone(), 2, 3).expect("upload fixture");
        let dir = staged.directory.clone();
        assert_eq!(
            fs::metadata(&dir).expect("upload fixture").mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(dir.join(".part"))
                .expect("upload fixture")
                .mode()
                & 0o777,
            0o600
        );
        assert!(create(root.clone(), 2, 3).is_err());
        assert!(staged.write_chunk(1, b"abc").is_err());
        assert!(staged.write_chunk(0, b"abcd").is_err());
        staged.write_chunk(0, b"abc").expect("upload fixture");
        assert_eq!(staged.publish().expect("upload fixture").size(), 3);
        assert_eq!(
            fs::read(dir.join("file.pdf")).expect("upload fixture"),
            b"abc"
        );
        let cancelled = create(root.clone(), 3, 3).expect("upload fixture");
        let cancelled_dir = cancelled.directory.clone();
        drop(cancelled);
        assert!(!cancelled_dir.exists());
        assert!(dir.join("file.pdf").exists());
        assert_eq!(
            create(root, 4, 0)
                .expect("upload fixture")
                .publish()
                .expect("upload fixture")
                .size(),
            0
        );
    }

    #[test]
    fn managed_symlinks_are_rejected() {
        let tmp = tempfile::tempdir().expect("upload fixture");
        let root = tmp.path().join("uploads");
        symlink(tmp.path(), &root).expect("upload fixture");
        assert!(create(root, 1, 1).is_err());
    }
}
