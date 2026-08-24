//! Private release-asset assembly entry; never shipped in product archives.

mod assets;

use std::env;
use std::path::PathBuf;

use anyhow::{Result, bail};

fn main() {
    if let Err(error) = run() {
        eprintln!("zterm release tool failed: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut arguments = env::args_os();
    let _program = arguments.next();
    let Some(command) = arguments.next() else {
        return usage();
    };
    match command.to_str() {
        Some("archive") => {
            let binary = required_path(&mut arguments, "binary")?;
            let output = required_path(&mut arguments, "output archive")?;
            require_end(arguments)?;
            assets::create_archive(&binary, &output)
        }
        Some("prepare") => {
            let input = required_path(&mut arguments, "input directory")?;
            let output = required_path(&mut arguments, "output directory")?;
            let tag = required_text(&mut arguments, "tag")?;
            let commit = required_text(&mut arguments, "source commit")?;
            let released_at = required_text(&mut arguments, "release timestamp")?;
            require_end(arguments)?;
            assets::prepare(&input, &output, &tag, &commit, &released_at)
        }
        Some("sign") => {
            let directory = required_path(&mut arguments, "release directory")?;
            require_end(arguments)?;
            assets::sign(&directory)
        }
        Some("derive-public-key") => {
            require_end(arguments)?;
            assets::derive_public_key()
        }
        Some("verify") => {
            let directory = required_path(&mut arguments, "release directory")?;
            require_end(arguments)?;
            assets::verify(&directory, true)
        }
        Some("verify-unsigned") => {
            let directory = required_path(&mut arguments, "release directory")?;
            require_end(arguments)?;
            assets::verify(&directory, false)
        }
        Some("render-test-installer") => {
            let directory = required_path(&mut arguments, "signed release directory")?;
            let output = required_path(&mut arguments, "fixture installer")?;
            require_end(arguments)?;
            assets::render_test_installer(&directory, &output)
        }
        _ => usage(),
    }
}

fn required_path(arguments: &mut env::ArgsOs, label: &str) -> Result<PathBuf> {
    arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("missing {label}"))
}

fn required_text(arguments: &mut env::ArgsOs, label: &str) -> Result<String> {
    arguments
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing {label}"))?
        .into_string()
        .map_err(|_| anyhow::anyhow!("{label} must be UTF-8"))
}

fn require_end(mut arguments: env::ArgsOs) -> Result<()> {
    if arguments.next().is_some() {
        bail!("unexpected extra argument");
    }
    Ok(())
}

fn usage<T>() -> Result<T> {
    bail!(
        "usage: zterm-release-tool <archive|prepare|sign|derive-public-key|verify|verify-unsigned|render-test-installer> ..."
    )
}
