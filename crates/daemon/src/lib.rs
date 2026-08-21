//! Side-effect-free daemon placeholder.
//!
//! This crate proves workspace dependency direction only. It does not start a
//! process, open a socket, read configuration, or create a terminal session.

/// Information printed by the Phase Zero CLI placeholder.
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
    /// Build-only protobuf schema version.
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
        schema_version: zterm_proto::SCHEMA_VERSION,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_reports_only_static_build_data() {
        let status = bootstrap_status();
        assert_eq!(status.phase, zterm_core::PHASE_NAME);
        assert_eq!(status.schema_version, zterm_core::BOOTSTRAP_SCHEMA_VERSION);
    }
}
