//! Isolated effective-user state fixture shared by daemon integration tests.

use std::fs;

use zterm_platform::user_state::UserPaths;

/// Temporary managed user-state paths.
pub struct TestState {
    _temporary: tempfile::TempDir,
    /// Product paths rooted inside the fixture.
    pub paths: UserPaths,
}

impl TestState {
    /// Creates an empty state fixture owned by the effective UID.
    pub fn new() -> Self {
        let temporary = tempfile::tempdir().expect("temporary state root");
        let home = temporary.path().join("home");
        fs::create_dir(&home).expect("test home");
        #[cfg(unix)]
        let uid = nix::unistd::Uid::effective().as_raw();
        #[cfg(not(unix))]
        let uid = 0;
        let paths = UserPaths::for_test(
            uid,
            home.clone(),
            home.join(".zterm"),
            temporary.path().join("run"),
        );
        Self {
            _temporary: temporary,
            paths,
        }
    }
}
