//! Reproducible protobuf code generation using a vendored `protoc` binary.

use std::{error::Error, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let proto_root = workspace_root.join("proto");
    let schemas = [
        "common.proto",
        "wire.proto",
        "local.proto",
        "pairing.proto",
        "device.proto",
        "transport.proto",
        "session.proto",
        "terminal.proto",
    ]
    .map(|name| proto_root.join("zterm/v1").join(name));
    let protoc = protoc_bin_vendored::protoc_bin_path()?;

    for schema in &schemas {
        println!("cargo:rerun-if-changed={}", schema.display());
    }

    let mut config = prost_build::Config::new();
    config.protoc_executable(protoc);
    // These generated values carry bearer tickets, nonces, proofs, opaque
    // payloads, terminal/input bytes, working directories, or route values.
    // Their Debug ownership is implemented manually in `src/lib.rs`; keep this
    // exact list in sync with the schemas rather than suppressing diagnostics
    // package-wide.
    config.skip_debug([
        ".zterm.v1.WireFrame",
        ".zterm.v1.PairTicketV1",
        ".zterm.v1.PairBegin",
        ".zterm.v1.PairChallenge",
        ".zterm.v1.PairProof",
        ".zterm.v1.PairAccepted",
        ".zterm.v1.LocalPairCreateResponse",
        ".zterm.v1.LocalPairAcceptRequest",
        ".zterm.v1.LocalSessionUnaryRequest",
        ".zterm.v1.LocalStatusResponse",
        ".zterm.v1.LocalValidateSetupRequest",
        ".zterm.v1.RelayRouteCacheV1",
        ".zterm.v1.SessionSummary",
        ".zterm.v1.SessionCreateRequest",
        ".zterm.v1.ResumeViewId",
        ".zterm.v1.TerminalAttachRequest",
        ".zterm.v1.TerminalSnapshot",
        ".zterm.v1.TerminalDelta",
        ".zterm.v1.TerminalInput",
    ]);
    config.compile_protos(&schemas, &[proto_root])?;

    Ok(())
}
