//! Reproducibly generated protobuf types.
//!
//! Phase Zero contains only a build probe. Terminal protocol messages are
//! intentionally out of scope.

/// Build-only protobuf messages.
pub mod bootstrap {
    /// First version of the build-only schema.
    pub mod v1 {
        include!(concat!(env!("OUT_DIR"), "/zterm.bootstrap.v1.rs"));
    }
}

/// Version shared by the source schema and the workspace identity.
pub const SCHEMA_VERSION: u32 = zterm_core::BOOTSTRAP_SCHEMA_VERSION;

#[cfg(test)]
mod tests {
    use prost::Message;

    use super::{SCHEMA_VERSION, bootstrap::v1::BuildProbe};

    #[test]
    fn vendored_protoc_generates_round_trippable_code() -> Result<(), prost::DecodeError> {
        let probe = BuildProbe {
            schema_version: SCHEMA_VERSION,
        };
        let decoded = BuildProbe::decode(probe.encode_to_vec().as_slice())?;

        assert_eq!(decoded, probe);
        Ok(())
    }
}
