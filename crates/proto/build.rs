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
    config.compile_protos(&schemas, &[proto_root])?;

    Ok(())
}
