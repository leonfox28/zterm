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
    // These generated values carry bearer tickets, nonces, proofs, or an
    // opaque payload which may contain any of them. Their Debug ownership is
    // implemented manually in `src/lib.rs`; keep this exact list in sync with
    // the pairing schema rather than suppressing diagnostics package-wide.
    config.skip_debug([
        ".zterm.v1.WireFrame",
        ".zterm.v1.PairTicketV1",
        ".zterm.v1.PairBegin",
        ".zterm.v1.PairChallenge",
        ".zterm.v1.PairProof",
        ".zterm.v1.PairAccepted",
        ".zterm.v1.LocalPairCreateResponse",
        ".zterm.v1.LocalPairAcceptRequest",
    ]);
    config.compile_protos(&schemas, &[proto_root])?;

    Ok(())
}
