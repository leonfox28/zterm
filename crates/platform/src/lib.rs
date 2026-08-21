//! Minimal platform boundary for the Phase Zero workspace.
//!
//! OS integration and daemon lifecycle are deliberately deferred.

/// Platform facts safe to expose from a side-effect-free placeholder.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlatformFacts {
    /// Rust's compile-time operating-system identifier.
    pub os: &'static str,
    /// Rust's compile-time CPU architecture identifier.
    pub arch: &'static str,
}

/// Returns compile-time platform facts without probing or mutating the host.
#[must_use]
pub const fn current() -> PlatformFacts {
    PlatformFacts {
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
    }
}

/// Returns the shared workspace build identity.
#[must_use]
pub const fn build_identity() -> zterm_core::BuildIdentity {
    zterm_core::BuildIdentity::current()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_platform_is_non_empty() {
        let platform = current();
        assert!(!platform.os.is_empty());
        assert!(!platform.arch.is_empty());
    }
}
