//! Shared zterm domain types and build identity.

pub mod terminal;

/// Human-readable name of the active implementation phase.
pub const PHASE_NAME: &str = "phase-one-foundation-gate";

/// Version of the build-only protobuf probe schema.
pub const BOOTSTRAP_SCHEMA_VERSION: u32 = 1;

/// Immutable identity exposed by the placeholder binaries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BuildIdentity {
    /// Cargo package version.
    pub version: &'static str,
    /// Current implementation phase.
    pub phase: &'static str,
}

impl BuildIdentity {
    /// Returns the identity for this workspace build.
    #[must_use]
    pub const fn current() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION"),
            phase: PHASE_NAME,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_identity_matches_workspace_constants() {
        let identity = BuildIdentity::current();
        assert_eq!(identity.phase, PHASE_NAME);
        assert_eq!(identity.version, env!("CARGO_PKG_VERSION"));
    }
}
