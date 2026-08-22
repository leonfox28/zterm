//! Effective operating-system account lookup shared by state and PTY owners.

use std::fmt;
use std::path::{Path, PathBuf};

/// Effective Unix account used by the running zterm process.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectiveAccount {
    uid: u32,
    gid: u32,
    home: PathBuf,
    shell: PathBuf,
}

impl EffectiveAccount {
    /// Looks up the effective UID in the operating-system account database.
    pub fn current() -> Result<Self, AccountError> {
        #[cfg(all(unix, not(any(target_os = "android", target_os = "redox"))))]
        {
            use nix::unistd::{Uid, User};

            let uid = Uid::effective();
            let user = User::from_uid(uid).map_err(|error| AccountError::Lookup {
                uid: uid.as_raw(),
                detail: error.to_string(),
            })?;
            let user = user.ok_or(AccountError::NotFound { uid: uid.as_raw() })?;
            Ok(Self {
                uid: uid.as_raw(),
                gid: user.gid.as_raw(),
                home: user.dir,
                shell: user.shell,
            })
        }

        #[cfg(not(all(unix, not(any(target_os = "android", target_os = "redox")))))]
        {
            Err(AccountError::UnsupportedPlatform)
        }
    }

    /// Constructs an explicit account for isolated library/integration tests.
    #[doc(hidden)]
    #[must_use]
    pub fn for_test(uid: u32, gid: u32, home: PathBuf, shell: PathBuf) -> Self {
        Self {
            uid,
            gid,
            home,
            shell,
        }
    }

    /// Effective numeric user identifier.
    #[must_use]
    pub const fn uid(&self) -> u32 {
        self.uid
    }

    /// Primary numeric group identifier from the account record.
    #[must_use]
    pub const fn gid(&self) -> u32 {
        self.gid
    }

    /// Absolute account home directory.
    #[must_use]
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// Absolute configured login shell.
    #[must_use]
    pub fn shell(&self) -> &Path {
        &self.shell
    }
}

/// Failure while resolving the effective account.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AccountError {
    /// Account database query failed.
    Lookup {
        /// Effective UID which was queried.
        uid: u32,
        /// Stable diagnostic detail.
        detail: String,
    },
    /// No account record exists for the effective UID.
    NotFound {
        /// Effective UID which was queried.
        uid: u32,
    },
    /// Effective-account lookup is not implemented on this target.
    UnsupportedPlatform,
}

impl fmt::Display for AccountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lookup { uid, detail } => {
                write!(formatter, "failed to read account {uid}: {detail}")
            }
            Self::NotFound { uid } => write!(formatter, "account {uid} was not found"),
            Self::UnsupportedPlatform => {
                write!(
                    formatter,
                    "effective-account lookup is unsupported on this platform"
                )
            }
        }
    }
}

impl std::error::Error for AccountError {}
