//! Reproducible protobuf code generation using a vendored `protoc` binary.

use std::{error::Error, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let proto_root = workspace_root.join("proto");
    let probe_schema = proto_root.join("zterm/bootstrap/v1/bootstrap.proto");
    let protoc = protoc_bin_vendored::protoc_bin_path()?;

    println!("cargo:rerun-if-changed={}", probe_schema.display());

    let mut config = prost_build::Config::new();
    config.protoc_executable(protoc);
    config.compile_protos(&[probe_schema], &[proto_root])?;

    Ok(())
}
