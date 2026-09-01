//! Private release-asset assembly entry; never shipped in product archives.

mod assets;

use std::env;
use std::path::PathBuf;

use anyhow::{Context, Result, bail, ensure};
use semver::Version;

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
        Some("validate-next-version") => {
            let version = required_text(&mut arguments, "next version")?;
            require_end(arguments)?;
            validate_next_version(&version, env!("CARGO_PKG_VERSION"))
        }
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

fn validate_next_version(candidate: &str, current: &str) -> Result<()> {
    let candidate_version = Version::parse(candidate).context("next version is not SemVer")?;
    ensure!(
        candidate == candidate_version.to_string(),
        "next version must use canonical SemVer text"
    );
    ensure!(
        candidate_version.build.is_empty(),
        "release versions must not contain SemVer build metadata because '+' is not an OCI tag character"
    );
    let current_version = Version::parse(current).context("workspace version is not SemVer")?;
    ensure!(
        candidate_version > current_version,
        "next version {candidate} must be newer than workspace version {current}"
    );
    println!("next release version {candidate} is valid");
    Ok(())
}

fn usage<T>() -> Result<T> {
    bail!(
        "usage: zterm-release-tool <validate-next-version|archive|prepare|sign|derive-public-key|verify|verify-unsigned|render-test-installer> ..."
    )
}

#[cfg(test)]
mod tests {
    use super::validate_next_version;

    #[test]
    fn next_version_accepts_newer_stable_and_prerelease_versions() {
        validate_next_version("0.1.10", "0.1.9").expect("newer patch release");
        validate_next_version("0.2.0-rc.1", "0.1.9").expect("newer prerelease");
    }

    #[test]
    fn next_version_rejects_same_downgrade_malformed_and_noncanonical_text() {
        for candidate in [
            "0.1.9",
            "0.1.8",
            "not-semver",
            "v0.2.0",
            "01.2.3",
            "0.2.0+build.1",
        ] {
            assert!(
                validate_next_version(candidate, "0.1.9").is_err(),
                "candidate must be rejected: {candidate}"
            );
        }
    }
}
