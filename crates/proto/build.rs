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
    .map(|name| proto_root.join("zterm/v2").join(name));
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
        ".zterm.v2.WireFrame",
        ".zterm.v2.PairTicketV1",
        ".zterm.v2.PairBegin",
        ".zterm.v2.PairChallenge",
        ".zterm.v2.PairProof",
        ".zterm.v2.PairAccepted",
        ".zterm.v2.LocalPairCreateResponse",
        ".zterm.v2.LocalPairAcceptRequest",
        ".zterm.v2.LocalSessionUnaryRequest",
        ".zterm.v2.LocalStatusResponse",
        ".zterm.v2.LocalValidateSetupRequest",
        ".zterm.v2.RelayRouteCacheV1",
        ".zterm.v2.SessionSummary",
        ".zterm.v2.SessionCreateRequest",
        ".zterm.v2.ResumeViewId",
        ".zterm.v2.TerminalAttachRequest",
        ".zterm.v2.TerminalInput",
        ".zterm.v2.TerminalClipboardWrite",
        ".zterm.v2.TerminalCell",
        ".zterm.v2.TerminalSurfaceRow",
        ".zterm.v2.TerminalSurface",
        ".zterm.v2.TerminalSemanticSnapshot",
        ".zterm.v2.TerminalSemanticRowPatch",
        ".zterm.v2.TerminalSemanticDelta",
        ".zterm.v2.TerminalSemanticHistoryWindowFrame",
    ]);
    config.compile_protos(&schemas, &[proto_root])?;

    Ok(())
}
