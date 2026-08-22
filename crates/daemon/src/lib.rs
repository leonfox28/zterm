//! Per-user state, local daemon lifecycle, terminal driver, and future Iroh adapter.
//!
//! Local readiness binds only a same-UID Unix socket. Iroh endpoint binding,
//! pairing, and session service remain later milestones.

pub mod bootstrap;
pub mod config;
pub mod error;
pub mod identity;
pub mod lifecycle;
pub mod local_ipc;
pub mod operations;
pub mod service;
pub mod store;
pub mod terminal_driver;
pub mod transport;

/// Static workspace build information.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BootstrapStatus {
    /// Cargo package version.
    pub version: &'static str,
    /// Current implementation phase.
    pub phase: &'static str,
    /// Compile-time operating system.
    pub os: &'static str,
    /// Compile-time CPU architecture.
    pub arch: &'static str,
    /// Persistent-state schema version.
    pub schema_version: u32,
}

/// Returns static build information without starting daemon behavior.
#[must_use]
pub const fn bootstrap_status() -> BootstrapStatus {
    let identity = zterm_core::BuildIdentity::current();
    let platform = zterm_platform::current();

    BootstrapStatus {
        version: identity.version,
        phase: identity.phase,
        os: platform.os,
        arch: platform.arch,
        schema_version: zterm_proto::STATE_SCHEMA_VERSION,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_status_reports_only_static_data() {
        let status = bootstrap_status();
        assert_eq!(status.phase, zterm_core::PHASE_NAME);
        assert_eq!(status.schema_version, zterm_core::STATE_SCHEMA_VERSION);
    }
}
