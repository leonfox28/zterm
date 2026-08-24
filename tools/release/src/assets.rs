//! Deterministic archive and authenticated Release asset ownership.

use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, Read, Write};
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};
use flate2::{Compression, GzBuilder, read::GzDecoder};
use ring::signature::{Ed25519KeyPair, KeyPair};
use semver::Version;
use serde_json::{Value, json};
use zeroize::Zeroizing;
use zterm_core::release::{
    MAX_RELEASE_ARTIFACT_BYTES, MAX_RELEASE_MANIFEST_BYTES, MINIMUM_GLIBC, MINIMUM_MACOS,
    RELEASE_BOOTSTRAP_SCHEMA, RELEASE_KEY_ID, RELEASE_MANIFEST_SCHEMA, RELEASE_ORIGIN,
    ReleaseArtifact, ReleaseClassification, ReleaseManifest, ReleaseSelfCheck,
    SUPPORTED_RELEASE_TARGETS, artifact_filename, immutable_asset_url, official_release_public_key,
    sha256_hex, sha256_reader, validate_unsigned_manifest, verify_official_release_manifest,
};
use zterm_core::{STATE_SCHEMA_VERSION, WIRE_MAJOR};

const MANIFEST_NAME: &str = "zterm-release.json";
const SIGNATURE_NAME: &str = "zterm-release.json.sig";
const INSTALLER_NAME: &str = "zterm-install.sh";
const SBOM_NAME: &str = "zterm-sbom.spdx.json";
const CHECKSUMS_NAME: &str = "SHA256SUMS";
const MAX_INSTALLER_BYTES: u64 = 256 * 1024;
const MAX_SBOM_BYTES: u64 = 4 * 1024 * 1024;
const MAX_BINARY_BYTES: u64 = 256 * 1024 * 1024;
const INSTALLER_TEMPLATE: &str = include_str!("../../../install/versioned.sh.in");

/// Creates a single-file deterministic native archive.
pub fn create_archive(binary: &Path, output: &Path) -> Result<()> {
    let epoch = std::env::var("SOURCE_DATE_EPOCH")
        .context("SOURCE_DATE_EPOCH is required for deterministic archives")?
        .parse::<u64>()
        .context("SOURCE_DATE_EPOCH must be an unsigned integer")?;
    create_archive_at_epoch(binary, output, epoch)
}

fn create_archive_at_epoch(binary: &Path, output: &Path, epoch: u64) -> Result<()> {
    let metadata = regular_file_metadata(binary, "release binary")?;
    ensure!(metadata.len() > 0, "release binary is empty");
    ensure!(
        metadata.len() <= MAX_BINARY_BYTES,
        "release binary exceeds its size bound"
    );
    ensure_executable(&metadata)?;
    let gzip_epoch = u32::try_from(epoch).context("SOURCE_DATE_EPOCH exceeds gzip range")?;

    let input = File::open(binary).context("unable to open release binary")?;
    let output_file = open_new(output, 0o644)?;
    let encoder = GzBuilder::new()
        .mtime(gzip_epoch)
        .write(output_file, Compression::best());
    let mut archive = tar::Builder::new(encoder);
    let mut header = tar::Header::new_gnu();
    header.set_size(metadata.len());
    header.set_mode(0o700);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(epoch);
    header.set_entry_type(tar::EntryType::Regular);
    header.set_cksum();
    archive
        .append_data(&mut header, "zterm", input)
        .context("unable to append release binary")?;
    let encoder = archive
        .into_inner()
        .context("unable to finish release archive")?;
    let output_file = encoder
        .finish()
        .context("unable to finish gzip release archive")?;
    output_file
        .sync_all()
        .context("unable to sync release archive")?;
    inspect_archive(output)
}

fn inspect_archive(path: &Path) -> Result<()> {
    let file = File::open(path).context("unable to open release archive")?;
    let decoder = GzDecoder::new(BufReader::new(file));
    let mut archive = tar::Archive::new(decoder);
    let mut entries = archive.entries().context("release archive is invalid")?;
    let Some(entry) = entries.next() else {
        bail!("release archive is empty");
    };
    let mut entry = entry.context("release archive entry is invalid")?;
    ensure!(
        entry
            .path()
            .context("release archive path is invalid")?
            .as_ref()
            == Path::new("zterm"),
        "release archive path is invalid"
    );
    let header = entry.header();
    ensure!(
        header.entry_type().is_file(),
        "release archive entry is not a regular file"
    );
    ensure!(
        header.mode().context("release archive mode is invalid")? == 0o700,
        "release archive mode is invalid"
    );
    ensure!(
        header.uid().context("release archive owner is invalid")? == 0
            && header.gid().context("release archive group is invalid")? == 0,
        "release archive ownership is invalid"
    );
    let size = header.size().context("release archive size is invalid")?;
    ensure!(
        size > 0 && size <= MAX_BINARY_BYTES,
        "release archive binary size is invalid"
    );
    io::copy(&mut entry, &mut io::sink()).context("release archive binary is truncated")?;
    ensure!(
        entries.next().is_none(),
        "release archive contains an unexpected entry"
    );
    Ok(())
}

/// Assembles all unsigned assets from four independently built native archives.
pub fn prepare(
    input: &Path,
    output: &Path,
    tag: &str,
    source_commit: &str,
    released_at: &str,
) -> Result<()> {
    ensure!(!output.exists(), "release output directory already exists");
    let version_text = tag
        .strip_prefix('v')
        .context("release tag must be v-prefixed")?;
    ensure!(
        version_text == env!("CARGO_PKG_VERSION"),
        "release tag does not match the Cargo workspace version"
    );
    let version = Version::parse(version_text).context("release tag is not canonical SemVer")?;
    ensure!(
        tag == format!("v{version}"),
        "release tag is not canonical SemVer"
    );
    let classification = ReleaseClassification::from_version(&version);

    let mut artifacts = Vec::with_capacity(SUPPORTED_RELEASE_TARGETS.len());
    for target in SUPPORTED_RELEASE_TARGETS {
        let filename = artifact_filename(target).context("unsupported release target")?;
        let archive_path = input.join(&filename);
        regular_file_metadata(&archive_path, "release archive")?;
        inspect_archive(&archive_path)?;
        let (length, digest) = digest_file(&archive_path, MAX_RELEASE_ARTIFACT_BYTES)?;
        let identity_path = input.join(format!("zterm-{target}.identity.json"));
        let identity_raw = read_bounded(&identity_path, MAX_RELEASE_MANIFEST_BYTES as u64)?;
        let self_check: ReleaseSelfCheck =
            serde_json::from_slice(&identity_raw).context("release identity JSON is invalid")?;
        ensure!(
            self_check.schema == RELEASE_MANIFEST_SCHEMA && self_check.product == "zterm",
            "release identity schema is invalid"
        );
        ensure!(
            self_check.build.version == version_text
                && self_check.build.target == target
                && self_check.build.source_commit == source_commit
                && self_check.build.wire_major == WIRE_MAJOR
                && self_check.build.state_schema == STATE_SCHEMA_VERSION
                && self_check.build.release_key_id == RELEASE_KEY_ID
                && self_check.build.classification == classification,
            "release identity does not match the requested source"
        );
        let apple = target.ends_with("apple-darwin");
        artifacts.push(ReleaseArtifact {
            filename: filename.clone(),
            target: target.to_owned(),
            url: immutable_asset_url(tag, &filename),
            length,
            sha256: digest,
            minimum_macos: apple.then(|| MINIMUM_MACOS.to_owned()),
            minimum_glibc: (!apple).then(|| MINIMUM_GLIBC.to_owned()),
            build: self_check.build,
        });
    }

    let manifest = ReleaseManifest {
        schema: RELEASE_MANIFEST_SCHEMA,
        product: "zterm".to_owned(),
        version: version_text.to_owned(),
        tag: tag.to_owned(),
        classification,
        source_commit: source_commit.to_owned(),
        released_at: released_at.to_owned(),
        wire_major: WIRE_MAJOR,
        state_schema: STATE_SCHEMA_VERSION,
        bootstrap_schema: RELEASE_BOOTSTRAP_SCHEMA,
        public_key_id: RELEASE_KEY_ID.to_owned(),
        artifacts,
    };
    validate_unsigned_manifest(&manifest).context("assembled release manifest is invalid")?;
    let raw_manifest =
        serde_json::to_vec(&manifest).context("unable to encode release manifest")?;
    ensure!(
        raw_manifest.len() <= MAX_RELEASE_MANIFEST_BYTES,
        "assembled release manifest exceeds its size bound"
    );
    let installer = render_installer(&manifest, &raw_manifest, false)?;
    let sbom = generate_sbom(&manifest)?;

    fs::create_dir(output).context("unable to create release output directory")?;
    for artifact in &manifest.artifacts {
        copy_new(
            &input.join(&artifact.filename),
            &output.join(&artifact.filename),
        )?;
    }
    write_new(&output.join(MANIFEST_NAME), &raw_manifest, 0o644)?;
    write_new(&output.join(INSTALLER_NAME), installer.as_bytes(), 0o755)?;
    write_new(&output.join(SBOM_NAME), &sbom, 0o644)?;
    verify(output, false)
}

fn render_installer(
    manifest: &ReleaseManifest,
    raw_manifest: &[u8],
    test_mode: bool,
) -> Result<String> {
    let mut cases = String::new();
    for target in SUPPORTED_RELEASE_TARGETS {
        let artifact = manifest
            .artifact_for_target(target)
            .context("release manifest target is missing")?;
        cases.push_str(&format!(
            "    {target}) archive_name='{}'; archive_length='{}'; archive_sha256='{}' ;;\n",
            artifact.filename, artifact.length, artifact.sha256
        ));
    }
    let base_url = format!("{RELEASE_ORIGIN}/download/{}", manifest.tag);
    let mut rendered = INSTALLER_TEMPLATE.to_owned();
    replace_token(&mut rendered, "@TAG@", &manifest.tag)?;
    replace_token(&mut rendered, "@BASE_URL@", &base_url)?;
    replace_token(
        &mut rendered,
        "@MANIFEST_SHA256@",
        &sha256_hex(raw_manifest),
    )?;
    replace_token(
        &mut rendered,
        "@TEST_MODE@",
        if test_mode { "1" } else { "0" },
    )?;
    replace_token(&mut rendered, "@ARTIFACT_CASES@", cases.trim_end())?;
    ensure!(
        !rendered.contains('@'),
        "generated installer has an unresolved token"
    );
    ensure!(
        rendered.len() <= usize::try_from(MAX_INSTALLER_BYTES).unwrap_or(usize::MAX),
        "generated installer exceeds its size bound"
    );
    Ok(rendered)
}

fn replace_token(output: &mut String, token: &str, value: &str) -> Result<()> {
    ensure!(
        output.matches(token).count() == 1,
        "installer template token is missing or duplicated"
    );
    *output = output.replacen(token, value, 1);
    Ok(())
}

/// Signs the exact prepared manifest and produces the final checksum inventory.
pub fn sign(directory: &Path) -> Result<()> {
    verify(directory, false)?;
    let manifest = read_bounded(
        &directory.join(MANIFEST_NAME),
        MAX_RELEASE_MANIFEST_BYTES as u64,
    )?;
    let secret_text = Zeroizing::new(
        std::env::var("ZTERM_RELEASE_SIGNING_KEY")
            .map_err(|_| anyhow::anyhow!("release signing key secret is unavailable"))?,
    );
    let seed = Zeroizing::new(decode_seed(secret_text.trim())?);
    let pair = Ed25519KeyPair::from_seed_unchecked(seed.as_ref())
        .map_err(|_| anyhow::anyhow!("release signing key is invalid"))?;
    let reviewed_public_key = official_release_public_key()
        .context("reviewed production release public key is unavailable")?;
    ensure!(
        pair.public_key().as_ref() == reviewed_public_key.as_slice(),
        "release signing key does not match reviewed production public key"
    );

    let signature = pair.sign(&manifest);
    write_new(&directory.join(SIGNATURE_NAME), signature.as_ref(), 0o644)?;
    let checksums = render_checksums(directory)?;
    write_new(&directory.join(CHECKSUMS_NAME), checksums.as_bytes(), 0o644)?;
    verify(directory, true)
}

/// Verifies an exact unsigned or final signed Release directory without execution.
pub fn verify(directory: &Path, signed: bool) -> Result<()> {
    ensure_inventory(directory, signed)?;
    let raw_manifest = read_bounded(
        &directory.join(MANIFEST_NAME),
        MAX_RELEASE_MANIFEST_BYTES as u64,
    )?;
    let manifest = if signed {
        let signature = read_bounded(&directory.join(SIGNATURE_NAME), 64)?;
        verify_official_release_manifest(&raw_manifest, &signature)
            .context("signed release manifest verification failed")?
    } else {
        let manifest: ReleaseManifest = serde_json::from_slice(&raw_manifest)
            .context("unsigned release manifest JSON is invalid")?;
        validate_unsigned_manifest(&manifest).context("unsigned release manifest is invalid")?;
        manifest
    };

    ensure!(
        manifest.version == env!("CARGO_PKG_VERSION"),
        "release manifest version does not match this source checkout"
    );
    for artifact in &manifest.artifacts {
        let path = directory.join(&artifact.filename);
        inspect_archive(&path)?;
        let (length, digest) = digest_file(&path, MAX_RELEASE_ARTIFACT_BYTES)?;
        ensure!(
            length == artifact.length && digest == artifact.sha256,
            "release archive does not match the manifest"
        );
    }

    let expected_installer = render_installer(&manifest, &raw_manifest, false)?;
    let installer = read_bounded(&directory.join(INSTALLER_NAME), MAX_INSTALLER_BYTES)?;
    ensure!(
        installer == expected_installer.as_bytes(),
        "generated installer does not match the release manifest"
    );
    verify_sbom(directory, &manifest)?;
    if signed {
        let expected_checksums = render_checksums(directory)?;
        let checksums = read_bounded(&directory.join(CHECKSUMS_NAME), MAX_INSTALLER_BYTES)?;
        ensure!(
            checksums == expected_checksums.as_bytes(),
            "release checksums are incomplete or invalid"
        );
    }
    Ok(())
}

/// Creates a fixture-only installer after authenticating the complete signed inventory.
pub fn render_test_installer(directory: &Path, output: &Path) -> Result<()> {
    verify(directory, true)?;
    let raw_manifest = read_bounded(
        &directory.join(MANIFEST_NAME),
        MAX_RELEASE_MANIFEST_BYTES as u64,
    )?;
    let signature = read_bounded(&directory.join(SIGNATURE_NAME), 64)?;
    let manifest = verify_official_release_manifest(&raw_manifest, &signature)
        .context("signed release manifest verification failed")?;
    let installer = render_installer(&manifest, &raw_manifest, true)?;
    write_new(output, installer.as_bytes(), 0o755)
}

fn ensure_inventory(directory: &Path, signed: bool) -> Result<()> {
    let expected = expected_inventory(signed)?;
    let mut observed = BTreeSet::new();
    for entry in fs::read_dir(directory).context("unable to read release directory")? {
        let entry = entry.context("unable to read release directory entry")?;
        ensure!(
            entry
                .file_type()
                .context("unable to inspect release asset")?
                .is_file(),
            "release directory contains a non-file entry"
        );
        observed.insert(
            entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("release asset name must be UTF-8"))?,
        );
    }
    ensure!(
        observed == expected,
        "release asset inventory is incomplete or unexpected"
    );
    Ok(())
}

fn expected_inventory(signed: bool) -> Result<BTreeSet<String>> {
    let mut names = BTreeSet::new();
    for target in SUPPORTED_RELEASE_TARGETS {
        names.insert(artifact_filename(target).context("unsupported release target")?);
    }
    names.insert(MANIFEST_NAME.to_owned());
    names.insert(INSTALLER_NAME.to_owned());
    names.insert(SBOM_NAME.to_owned());
    if signed {
        names.insert(SIGNATURE_NAME.to_owned());
        names.insert(CHECKSUMS_NAME.to_owned());
    }
    Ok(names)
}

fn render_checksums(directory: &Path) -> Result<String> {
    let mut output = String::new();
    for name in expected_inventory(true)? {
        if name == CHECKSUMS_NAME {
            continue;
        }
        let (_, digest) = digest_file(&directory.join(&name), MAX_RELEASE_ARTIFACT_BYTES)?;
        output.push_str(&format!("{digest}  {name}\n"));
    }
    Ok(output)
}

fn decode_seed(value: &str) -> Result<[u8; 32]> {
    ensure!(
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "release signing key must be 64 lowercase hexadecimal characters"
    );
    let mut seed = [0_u8; 32];
    for (index, chunk) in value.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        seed[index] = hex_value(chunk[0]) * 16 + hex_value(chunk[1]);
    }
    Ok(seed)
}

fn hex_value(value: u8) -> u8 {
    match value {
        b'0'..=b'9' => value - b'0',
        b'a'..=b'f' => value - b'a' + 10,
        _ => 0,
    }
}

fn generate_sbom(manifest: &ReleaseManifest) -> Result<Vec<u8>> {
    let repository_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let metadata = Command::new("cargo")
        .args(["metadata", "--locked", "--format-version", "1"])
        .current_dir(repository_root)
        .output()
        .context("unable to run cargo metadata for the SBOM")?;
    ensure!(
        metadata.status.success(),
        "cargo metadata failed for the SBOM"
    );
    let metadata: Value =
        serde_json::from_slice(&metadata.stdout).context("cargo metadata output is invalid")?;
    let package_values = metadata["packages"]
        .as_array()
        .context("cargo metadata omitted packages")?;
    ensure!(
        !package_values.is_empty(),
        "cargo metadata package list is empty"
    );

    let mut packages = Vec::with_capacity(package_values.len());
    for package in package_values {
        let name = package["name"]
            .as_str()
            .context("cargo metadata package name is invalid")?;
        let version = package["version"]
            .as_str()
            .context("cargo metadata package version is invalid")?;
        let license = package["license"].as_str().unwrap_or("NOASSERTION");
        packages.push((name.to_owned(), version.to_owned(), license.to_owned()));
    }
    packages.sort();
    let spdx_packages = packages
        .iter()
        .enumerate()
        .map(|(index, (name, version, license))| {
            json!({
                "SPDXID": format!("SPDXRef-Package-{index}-{}", spdx_component(name)),
                "name": name,
                "versionInfo": version,
                "downloadLocation": "NOASSERTION",
                "filesAnalyzed": false,
                "licenseConcluded": "NOASSERTION",
                "licenseDeclared": license,
                "copyrightText": "NOASSERTION"
            })
        })
        .collect::<Vec<_>>();
    let document = json!({
        "spdxVersion": "SPDX-2.3",
        "dataLicense": "CC0-1.0",
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": format!("zterm-{}", manifest.version),
        "documentNamespace": format!(
            "https://github.com/leonfox28/zterm/releases/download/{}/{}#{}",
            manifest.tag, SBOM_NAME, manifest.source_commit
        ),
        "creationInfo": {
            "created": manifest.released_at,
            "creators": [format!("Tool: zterm-release-tool-{}", env!("CARGO_PKG_VERSION"))]
        },
        "comment": format!("zterm source commit {}", manifest.source_commit),
        "packages": spdx_packages
    });
    let encoded = serde_json::to_vec(&document).context("unable to encode SPDX SBOM")?;
    ensure!(
        encoded.len() <= usize::try_from(MAX_SBOM_BYTES).unwrap_or(usize::MAX),
        "SPDX SBOM exceeds its size bound"
    );
    Ok(encoded)
}

fn verify_sbom(directory: &Path, manifest: &ReleaseManifest) -> Result<()> {
    let raw = read_bounded(&directory.join(SBOM_NAME), MAX_SBOM_BYTES)?;
    let sbom: Value = serde_json::from_slice(&raw).context("SPDX SBOM JSON is invalid")?;
    ensure!(
        sbom["spdxVersion"] == "SPDX-2.3",
        "SPDX SBOM version is invalid"
    );
    ensure!(
        sbom["name"] == format!("zterm-{}", manifest.version),
        "SPDX SBOM product version is invalid"
    );
    ensure!(
        sbom["creationInfo"]["created"] == manifest.released_at,
        "SPDX SBOM timestamp is invalid"
    );
    ensure!(
        sbom["comment"] == format!("zterm source commit {}", manifest.source_commit),
        "SPDX SBOM source commit is invalid"
    );
    ensure!(
        sbom["packages"]
            .as_array()
            .is_some_and(|items| !items.is_empty()),
        "SPDX SBOM package inventory is empty"
    );
    Ok(())
}

fn spdx_component(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-') {
                character
            } else {
                '-'
            }
        })
        .collect()
}

fn digest_file(path: &Path, maximum: u64) -> Result<(u64, String)> {
    let file = File::open(path).context("unable to open release asset")?;
    sha256_reader(BufReader::new(file), maximum).context("unable to digest release asset")
}

fn read_bounded(path: &Path, maximum: u64) -> Result<Vec<u8>> {
    let metadata = regular_file_metadata(path, "release asset")?;
    ensure!(
        metadata.len() <= maximum,
        "release asset exceeds its size bound"
    );
    let capacity = usize::try_from(metadata.len()).context("release asset is too large")?;
    let mut bytes = Vec::with_capacity(capacity);
    File::open(path)
        .context("unable to open release asset")?
        .take(maximum.saturating_add(1))
        .read_to_end(&mut bytes)
        .context("unable to read release asset")?;
    ensure!(
        u64::try_from(bytes.len()).unwrap_or(u64::MAX) <= maximum,
        "release asset exceeds its size bound"
    );
    Ok(bytes)
}

fn regular_file_metadata(path: &Path, label: &str) -> Result<fs::Metadata> {
    let metadata =
        fs::symlink_metadata(path).with_context(|| format!("unable to inspect {label}"))?;
    ensure!(
        metadata.file_type().is_file(),
        "{label} is not a regular file"
    );
    Ok(metadata)
}

fn copy_new(source: &Path, destination: &Path) -> Result<()> {
    let mut input = File::open(source).context("unable to open release archive")?;
    let mut output = open_new(destination, 0o644)?;
    io::copy(&mut input, &mut output).context("unable to copy release archive")?;
    output.sync_all().context("unable to sync release archive")
}

fn write_new(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let mut output = open_new(path, mode)?;
    output
        .write_all(bytes)
        .context("unable to write release asset")?;
    output.sync_all().context("unable to sync release asset")
}

fn open_new(path: &Path, mode: u32) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    configure_mode(&mut options, mode);
    options.open(path).context("unable to create release asset")
}

#[cfg(unix)]
fn configure_mode(options: &mut OpenOptions, mode: u32) {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(mode);
}

#[cfg(not(unix))]
fn configure_mode(_options: &mut OpenOptions, _mode: u32) {}

#[cfg(unix)]
fn ensure_executable(metadata: &fs::Metadata) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    ensure!(
        metadata.permissions().mode() & 0o111 != 0,
        "release binary is not executable"
    );
    Ok(())
}

#[cfg(not(unix))]
fn ensure_executable(_metadata: &fs::Metadata) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use zterm_core::release::ReleaseBuildIdentity;

    #[test]
    fn deterministic_archive_has_the_formal_inventory() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let binary = temporary.path().join("candidate");
        fs::write(&binary, b"candidate bytes").expect("candidate write");
        make_executable(&binary);
        let first = temporary.path().join("first.tar.gz");
        let second = temporary.path().join("second.tar.gz");

        create_archive_at_epoch(&binary, &first, 1_700_000_000).expect("first archive");
        create_archive_at_epoch(&binary, &second, 1_700_000_000).expect("second archive");

        assert_eq!(
            fs::read(first).expect("first bytes"),
            fs::read(second).expect("second bytes")
        );
    }

    #[test]
    fn installer_is_derived_from_the_manifest_inventory() {
        let manifest = fixture_manifest();
        let raw = serde_json::to_vec(&manifest).expect("manifest JSON");

        let installer = render_installer(&manifest, &raw, false).expect("installer");

        assert!(installer.contains("release_tag='v0.1.2'"));
        assert!(installer.contains("test_mode='0'"));
        for target in SUPPORTED_RELEASE_TARGETS {
            assert!(installer.contains(target));
            assert!(installer.contains(&artifact_filename(target).expect("filename")));
        }
        assert!(!installer.contains('@'));
        let mut syntax = Command::new("sh")
            .arg("-n")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .expect("POSIX shell");
        syntax
            .stdin
            .take()
            .expect("shell stdin")
            .write_all(installer.as_bytes())
            .expect("installer syntax input");
        assert!(syntax.wait().expect("shell syntax status").success());
    }

    fn fixture_manifest() -> ReleaseManifest {
        let source_commit = "0123456789abcdef0123456789abcdef01234567";
        let artifacts = SUPPORTED_RELEASE_TARGETS
            .iter()
            .map(|target| {
                let filename = artifact_filename(target).expect("filename");
                let apple = target.ends_with("apple-darwin");
                ReleaseArtifact {
                    filename: filename.clone(),
                    target: (*target).to_owned(),
                    url: immutable_asset_url("v0.1.2", &filename),
                    length: 42,
                    sha256: "ab".repeat(32),
                    minimum_macos: apple.then(|| MINIMUM_MACOS.to_owned()),
                    minimum_glibc: (!apple).then(|| MINIMUM_GLIBC.to_owned()),
                    build: ReleaseBuildIdentity {
                        version: "0.1.2".to_owned(),
                        target: (*target).to_owned(),
                        source_commit: source_commit.to_owned(),
                        wire_major: WIRE_MAJOR,
                        state_schema: STATE_SCHEMA_VERSION,
                        release_key_id: RELEASE_KEY_ID.to_owned(),
                        classification: ReleaseClassification::Stable,
                    },
                }
            })
            .collect();
        ReleaseManifest {
            schema: RELEASE_MANIFEST_SCHEMA,
            product: "zterm".to_owned(),
            version: "0.1.2".to_owned(),
            tag: "v0.1.2".to_owned(),
            classification: ReleaseClassification::Stable,
            source_commit: source_commit.to_owned(),
            released_at: "2026-08-24T12:00:00Z".to_owned(),
            wire_major: WIRE_MAJOR,
            state_schema: STATE_SCHEMA_VERSION,
            bootstrap_schema: RELEASE_BOOTSTRAP_SCHEMA,
            public_key_id: RELEASE_KEY_ID.to_owned(),
            artifacts,
        }
    }

    #[cfg(unix)]
    fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path).expect("metadata").permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(path, permissions).expect("permissions");
    }

    #[cfg(not(unix))]
    fn make_executable(_path: &Path) {}
}
