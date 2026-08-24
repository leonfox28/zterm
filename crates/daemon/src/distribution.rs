//! Bounded official Release fetching, candidate verification, and install metadata.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use flate2::read::GzDecoder;
use semver::Version;
use serde::{Deserialize, Serialize};
use tar::Archive;
use tempfile::TempDir;
use zterm_core::release::{
    MAX_RELEASE_MANIFEST_BYTES, RELEASE_ORIGIN, RELEASE_SIGNATURE_BYTES, ReleaseArtifact,
    ReleaseClassification, ReleaseError, ReleaseManifest, ReleaseSelfCheck,
    official_release_public_key, require_official_distribution_build, sha256_hex,
    verify_release_manifest,
};
use zterm_core::{BuildIdentity, DomainErrorKind};
use zterm_platform::user_state::{UserPaths, atomic_write};

use crate::error::DaemonError;

const MANIFEST_ASSET: &str = "zterm-release.json";
const SIGNATURE_ASSET: &str = "zterm-release.json.sig";
const MAX_CANDIDATE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_SELF_CHECK_BYTES: u64 = 16 * 1024;
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const HTTP_TOTAL_TIMEOUT: Duration = Duration::from_secs(60);
const CANDIDATE_CHECK_TIMEOUT: Duration = Duration::from_secs(5);
const INSTALL_METADATA_SCHEMA: u32 = 1;

/// Stable or exact immutable Release selection from a public update command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReleaseSelection {
    /// GitHub's latest non-draft, non-prerelease Release.
    LatestStable,
    /// One canonical `v` plus SemVer tag, including explicit prereleases.
    Exact(String),
}

impl ReleaseSelection {
    /// Parses an optional exact tag without accepting a branch, commit, or URL.
    pub fn parse(tag: Option<&str>) -> Result<Self, DistributionError> {
        let Some(tag) = tag else {
            return Ok(Self::LatestStable);
        };
        let version = tag
            .strip_prefix('v')
            .ok_or(DistributionError::InvalidSelection)
            .and_then(|value| {
                Version::parse(value).map_err(|_| DistributionError::InvalidSelection)
            })?;
        if tag != format!("v{version}") {
            return Err(DistributionError::InvalidSelection);
        }
        Ok(Self::Exact(tag.to_owned()))
    }

    fn asset_url(&self, asset: &str) -> String {
        match self {
            Self::LatestStable => format!("{RELEASE_ORIGIN}/latest/download/{asset}"),
            Self::Exact(tag) => format!("{RELEASE_ORIGIN}/download/{tag}/{asset}"),
        }
    }
}

/// Prepared candidate whose bytes and identity are authenticated before daemon contact.
pub struct PreparedRelease {
    _temporary: TempDir,
    #[cfg(unix)]
    candidate: PathBuf,
    manifest: ReleaseManifest,
    artifact: ReleaseArtifact,
}

impl fmt::Debug for PreparedRelease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedRelease")
            .field("version", &self.manifest.version)
            .field("target", &self.artifact.target)
            .field("temporary", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

impl PreparedRelease {
    /// Authenticated candidate version.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.manifest.version
    }

    /// Authenticated candidate target.
    #[must_use]
    pub fn target(&self) -> &str {
        &self.artifact.target
    }

    /// Candidate file retained by this owner until activation completes.
    #[cfg(unix)]
    #[must_use]
    pub(crate) fn candidate(&self) -> &Path {
        &self.candidate
    }

    /// Authenticated manifest retained for post-activation metadata.
    #[cfg(unix)]
    #[must_use]
    pub(crate) const fn manifest(&self) -> &ReleaseManifest {
        &self.manifest
    }
}

/// Content-free failure from the download/candidate boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DistributionError {
    /// Exact version selection was not canonical `v` plus SemVer.
    InvalidSelection,
    /// Production release public key has not been configured.
    PublicKeyUnavailable,
    /// Fixed HTTPS fetch failed, redirected excessively, or crossed a bound.
    Fetch,
    /// Signed manifest failed schema or identity validation.
    Manifest,
    /// Detached manifest signature failed authentication.
    Signature,
    /// Downloaded archive did not match its authenticated length or SHA-256.
    Artifact,
    /// Archive inventory, type, or decompressed bound was invalid.
    Archive,
    /// Candidate process failed its side-effect-free self-check.
    Candidate,
    /// Candidate is not newer than the running CLI.
    NotNewer,
    /// A local private staging operation failed.
    Staging,
}

impl fmt::Display for DistributionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidSelection => "release selection must be a canonical v-prefixed SemVer tag",
            Self::PublicKeyUnavailable => "official release verification key is not configured",
            Self::Fetch => "unable to download bounded bytes from the official Release endpoint",
            Self::Manifest => "signed release manifest is invalid",
            Self::Signature => "release manifest signature is invalid",
            Self::Artifact => "release archive does not match authenticated metadata",
            Self::Archive => "release archive inventory is invalid",
            Self::Candidate => "release candidate self-check failed",
            Self::NotNewer => "release candidate is not newer than this zterm binary",
            Self::Staging => "unable to stage the verified release candidate",
        })
    }
}

impl std::error::Error for DistributionError {}

impl From<DistributionError> for DaemonError {
    fn from(error: DistributionError) -> Self {
        let kind = match error {
            DistributionError::PublicKeyUnavailable => DomainErrorKind::ReleaseSignatureInvalid,
            DistributionError::Signature => DomainErrorKind::ReleaseSignatureInvalid,
            DistributionError::Manifest | DistributionError::InvalidSelection => {
                DomainErrorKind::ReleaseManifestInvalid
            }
            DistributionError::Artifact
            | DistributionError::Archive
            | DistributionError::Candidate => DomainErrorKind::ReleaseArtifactInvalid,
            DistributionError::Fetch => DomainErrorKind::ReleaseUnavailable,
            DistributionError::NotNewer => DomainErrorKind::UpdateRejected,
            DistributionError::Staging => DomainErrorKind::PathUnsafe,
        };
        Self::new(kind, error.to_string())
    }
}

trait ReleaseFetcher {
    fn fetch(&self, url: &str, maximum: u64) -> Result<Vec<u8>, DistributionError>;
}

struct HttpReleaseFetcher {
    client: reqwest::blocking::Client,
}

impl HttpReleaseFetcher {
    fn new() -> Result<Self, DistributionError> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(HTTP_CONNECT_TIMEOUT)
            .timeout(HTTP_TOTAL_TIMEOUT)
            .https_only(true)
            .redirect(reqwest::redirect::Policy::limited(5))
            .user_agent(concat!("zterm/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| DistributionError::Fetch)?;
        Ok(Self { client })
    }
}

impl ReleaseFetcher for HttpReleaseFetcher {
    fn fetch(&self, url: &str, maximum: u64) -> Result<Vec<u8>, DistributionError> {
        let mut response = self
            .client
            .get(url)
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .map_err(|_| DistributionError::Fetch)?;
        if response
            .content_length()
            .is_some_and(|length| length > maximum)
        {
            return Err(DistributionError::Fetch);
        }
        let capacity = usize::try_from(maximum.min(1024 * 1024)).unwrap_or(0);
        let mut bytes = Vec::with_capacity(capacity);
        response
            .by_ref()
            .take(maximum.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|_| DistributionError::Fetch)?;
        if u64::try_from(bytes.len()).map_or(true, |length| length > maximum) {
            return Err(DistributionError::Fetch);
        }
        Ok(bytes)
    }
}

/// Downloads and authenticates a release candidate on a blocking worker.
pub async fn prepare_update(selection: ReleaseSelection) -> Result<PreparedRelease, DaemonError> {
    tokio::task::spawn_blocking(move || {
        let fetcher = HttpReleaseFetcher::new()?;
        let public_key = official_release_public_key().map_err(map_release_error)?;
        prepare_with(&fetcher, &selection, &public_key, BuildIdentity::current())
    })
    .await
    .map_err(|_| DaemonError::from(DistributionError::Staging))?
    .map_err(Into::into)
}

/// Produces the current side-effect-free self-check JSON.
pub fn self_check_json() -> Result<String, DaemonError> {
    serde_json::to_string(&ReleaseSelfCheck::current())
        .map(|json| format!("{json}\n"))
        .map_err(|_| DaemonError::from(DistributionError::Candidate))
}

/// Verifies exact manifest/signature files against this binary's embedded identity.
pub fn verify_candidate_files(manifest: &Path, signature: &Path) -> Result<(), DaemonError> {
    let manifest = read_bounded_file(
        manifest,
        u64::try_from(MAX_RELEASE_MANIFEST_BYTES).unwrap_or(u64::MAX),
    )?;
    let signature = read_bounded_file(
        signature,
        u64::try_from(RELEASE_SIGNATURE_BYTES).unwrap_or(u64::MAX),
    )?;
    let verified = zterm_core::release::verify_official_release_manifest(&manifest, &signature)
        .map_err(map_release_error)
        .map_err(DaemonError::from)?;
    verified
        .require_build_identity(&BuildIdentity::current())
        .map(|_| ())
        .map_err(map_release_error)
        .map_err(Into::into)
}

/// Installs this already verified candidate without creating product state.
pub fn install_current_executable(destination: &Path) -> Result<(), DaemonError> {
    require_official_distribution_build(&BuildIdentity::current()).map_err(|_| {
        DaemonError::new(
            DomainErrorKind::PathUnsafe,
            "installation requires an official managed zterm Release build",
        )
    })?;
    let source = std::env::current_exe().map_err(|_| {
        DaemonError::new(
            DomainErrorKind::PathUnsafe,
            "unable to locate verified installer candidate",
        )
    })?;
    let paths = crate::lifecycle::production_user_paths()?;
    zterm_platform::user_state::install_executable(&source, destination, paths.uid())
        .map_err(|error| DaemonError::new(DomainErrorKind::PathUnsafe, error.to_string()))
}

/// Proves the exact executable is both locally safe and an official managed build.
pub(crate) fn validate_managed_executable(path: &Path, uid: u32) -> Result<(), DaemonError> {
    zterm_platform::user_state::validate_owned_executable(path, uid)
        .map_err(|error| DaemonError::new(DomainErrorKind::PathUnsafe, error.to_string()))?;
    require_official_distribution_build(&BuildIdentity::current()).map_err(|_| {
        DaemonError::new(
            DomainErrorKind::PathUnsafe,
            "destructive distribution operations require an official managed zterm Release build",
        )
    })
}

/// Re-runs the side-effect-free identity check against the activated path.
#[cfg(unix)]
pub(crate) fn verify_activated_candidate(
    candidate: &Path,
    manifest: &ReleaseManifest,
) -> Result<(), DaemonError> {
    run_candidate_self_check(candidate)
        .and_then(|self_check| {
            self_check
                .require_manifest(manifest)
                .map_err(map_release_error)
        })
        .map_err(Into::into)
}

/// Writes managed install metadata after setup or a successful activation.
pub fn write_install_metadata(
    paths: &UserPaths,
    executable: &Path,
    manifest: Option<&ReleaseManifest>,
) -> Result<(), DaemonError> {
    let build = BuildIdentity::current();
    let (version, target, source_commit, wire_major, state_schema, release_key_id, classification) =
        manifest.map_or_else(
            || {
                (
                    build.version.to_owned(),
                    build.target.to_owned(),
                    build.source_commit.to_owned(),
                    build.wire_major,
                    build.state_schema,
                    build.release_key_id.to_owned(),
                    build.release_classification.to_owned(),
                )
            },
            |manifest| {
                (
                    manifest.version.clone(),
                    build.target.to_owned(),
                    manifest.source_commit.clone(),
                    manifest.wire_major,
                    manifest.state_schema,
                    manifest.public_key_id.clone(),
                    manifest.classification.as_str().to_owned(),
                )
            },
        );
    let metadata = InstallMetadata {
        schema: INSTALL_METADATA_SCHEMA,
        version,
        target,
        source_commit,
        wire_major,
        state_schema,
        release_key_id,
        classification,
        executable: executable.to_string_lossy().into_owned(),
    };
    let raw =
        serde_json::to_vec(&metadata).map_err(|_| DaemonError::from(DistributionError::Staging))?;
    atomic_write(paths.install_metadata(), paths.uid(), |file| {
        file.write_all(&raw)
    })
    .map_err(|error| DaemonError::new(DomainErrorKind::PathUnsafe, error.to_string()))
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct InstallMetadata {
    schema: u32,
    version: String,
    target: String,
    source_commit: String,
    wire_major: u32,
    state_schema: u32,
    release_key_id: String,
    classification: String,
    executable: String,
}

fn prepare_with(
    fetcher: &impl ReleaseFetcher,
    selection: &ReleaseSelection,
    public_key: &[u8; 32],
    current: BuildIdentity,
) -> Result<PreparedRelease, DistributionError> {
    let raw_manifest = fetcher.fetch(
        &selection.asset_url(MANIFEST_ASSET),
        u64::try_from(MAX_RELEASE_MANIFEST_BYTES).unwrap_or(u64::MAX),
    )?;
    let signature = fetcher.fetch(
        &selection.asset_url(SIGNATURE_ASSET),
        u64::try_from(RELEASE_SIGNATURE_BYTES).unwrap_or(u64::MAX),
    )?;
    let manifest = verify_release_manifest(&raw_manifest, &signature, public_key)
        .map_err(map_release_error)?;
    match selection {
        ReleaseSelection::LatestStable
            if manifest.classification != ReleaseClassification::Stable =>
        {
            return Err(DistributionError::Manifest);
        }
        ReleaseSelection::Exact(tag) if tag != &manifest.tag => {
            return Err(DistributionError::Manifest);
        }
        ReleaseSelection::LatestStable | ReleaseSelection::Exact(_) => {}
    }
    manifest
        .require_newer_than(current.version)
        .map_err(map_release_error)?;
    let artifact = manifest
        .artifact_for_target(current.target)
        .map_err(map_release_error)?
        .clone();
    let archive = fetcher.fetch(&artifact.url, artifact.length)?;
    if u64::try_from(archive.len()).ok() != Some(artifact.length)
        || sha256_hex(&archive) != artifact.sha256
    {
        return Err(DistributionError::Artifact);
    }

    let temporary = tempfile::tempdir().map_err(|_| DistributionError::Staging)?;
    let candidate = extract_candidate(&archive, temporary.path())?;
    let manifest_path = temporary.path().join(MANIFEST_ASSET);
    let signature_path = temporary.path().join(SIGNATURE_ASSET);
    write_private(&manifest_path, &raw_manifest)?;
    write_private(&signature_path, &signature)?;
    let self_check = run_candidate_self_check(&candidate)?;
    self_check
        .require_manifest(&manifest)
        .map_err(map_release_error)?;
    run_candidate_verification(&candidate, &manifest_path, &signature_path)?;

    Ok(PreparedRelease {
        _temporary: temporary,
        #[cfg(unix)]
        candidate,
        manifest,
        artifact,
    })
}

fn extract_candidate(archive: &[u8], directory: &Path) -> Result<PathBuf, DistributionError> {
    let decoder = GzDecoder::new(Cursor::new(archive));
    let mut archive = Archive::new(decoder);
    let entries = archive.entries().map_err(|_| DistributionError::Archive)?;
    let candidate = directory.join("zterm");
    let mut found = false;
    for entry in entries {
        let mut entry = entry.map_err(|_| DistributionError::Archive)?;
        if found
            || entry
                .path()
                .map_err(|_| DistributionError::Archive)?
                .as_ref()
                != Path::new("zterm")
            || !entry.header().entry_type().is_file()
        {
            return Err(DistributionError::Archive);
        }
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o700);
        }
        let mut output = options
            .open(&candidate)
            .map_err(|_| DistributionError::Staging)?;
        let copied = io::copy(
            &mut entry.by_ref().take(MAX_CANDIDATE_BYTES + 1),
            &mut output,
        )
        .map_err(|_| DistributionError::Archive)?;
        if copied == 0 || copied > MAX_CANDIDATE_BYTES {
            return Err(DistributionError::Archive);
        }
        output.sync_all().map_err(|_| DistributionError::Staging)?;
        found = true;
    }
    if !found {
        return Err(DistributionError::Archive);
    }
    Ok(candidate)
}

fn run_candidate_self_check(candidate: &Path) -> Result<ReleaseSelfCheck, DistributionError> {
    let output = run_candidate(candidate, &["--internal-release-self-check"], true)?;
    serde_json::from_slice(&output).map_err(|_| DistributionError::Candidate)
}

fn run_candidate_verification(
    candidate: &Path,
    manifest: &Path,
    signature: &Path,
) -> Result<(), DistributionError> {
    let manifest = manifest.to_str().ok_or(DistributionError::Staging)?;
    let signature = signature.to_str().ok_or(DistributionError::Staging)?;
    run_candidate(
        candidate,
        &["--internal-release-verify", manifest, signature],
        false,
    )
    .map(|_| ())
}

fn run_candidate(
    candidate: &Path,
    arguments: &[&str],
    capture_stdout: bool,
) -> Result<Vec<u8>, DistributionError> {
    let mut output = tempfile::tempfile().map_err(|_| DistributionError::Staging)?;
    let stdout = if capture_stdout {
        Stdio::from(output.try_clone().map_err(|_| DistributionError::Staging)?)
    } else {
        Stdio::null()
    };
    let mut child = Command::new(candidate)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| DistributionError::Candidate)?;
    let deadline = Instant::now() + CANDIDATE_CHECK_TIMEOUT;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|_| DistributionError::Candidate)? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(DistributionError::Candidate);
        }
        thread::sleep(Duration::from_millis(10));
    };
    if !status.success() {
        return Err(DistributionError::Candidate);
    }
    if !capture_stdout {
        return Ok(Vec::new());
    }
    output
        .seek(SeekFrom::Start(0))
        .map_err(|_| DistributionError::Candidate)?;
    let mut bytes = Vec::new();
    output
        .take(MAX_SELF_CHECK_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| DistributionError::Candidate)?;
    if u64::try_from(bytes.len()).map_or(true, |length| length > MAX_SELF_CHECK_BYTES) {
        return Err(DistributionError::Candidate);
    }
    Ok(bytes)
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), DistributionError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|_| DistributionError::Staging)?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| DistributionError::Staging)
}

fn read_bounded_file(path: &Path, maximum: u64) -> Result<Vec<u8>, DaemonError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| DaemonError::from(DistributionError::Staging))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > maximum {
        return Err(DaemonError::from(DistributionError::Staging));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    File::open(path)
        .and_then(|file| file.take(maximum + 1).read_to_end(&mut bytes))
        .map_err(|_| DaemonError::from(DistributionError::Staging))?;
    if u64::try_from(bytes.len()).map_or(true, |length| length > maximum) {
        return Err(DaemonError::from(DistributionError::Staging));
    }
    Ok(bytes)
}

fn map_release_error(error: ReleaseError) -> DistributionError {
    match error {
        ReleaseError::PublicKeyUnavailable => DistributionError::PublicKeyUnavailable,
        ReleaseError::SignatureInvalid | ReleaseError::SignatureSize => {
            DistributionError::Signature
        }
        ReleaseError::ArtifactSize
        | ReleaseError::ArtifactDigest
        | ReleaseError::ArtifactLocation => DistributionError::Artifact,
        ReleaseError::NotNewer => DistributionError::NotNewer,
        ReleaseError::DigestRead => DistributionError::Artifact,
        ReleaseError::ManifestSize
        | ReleaseError::ManifestSyntax
        | ReleaseError::Schema
        | ReleaseError::ReleaseIdentity
        | ReleaseError::InvalidVersion
        | ReleaseError::SourceIdentity
        | ReleaseError::KeyIdentifier
        | ReleaseError::ArtifactInventory
        | ReleaseError::PlatformFloor
        | ReleaseError::UnsupportedTarget
        | ReleaseError::BuildIdentityMismatch => DistributionError::Manifest,
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::collections::BTreeMap;

    use ring::signature::{Ed25519KeyPair, KeyPair};
    use zterm_core::release::{
        MINIMUM_GLIBC, MINIMUM_MACOS, RELEASE_BOOTSTRAP_SCHEMA, RELEASE_KEY_ID,
        RELEASE_MANIFEST_SCHEMA, ReleaseBuildIdentity, SUPPORTED_RELEASE_TARGETS,
        artifact_filename, immutable_asset_url,
    };

    use super::*;

    struct MapFetcher(BTreeMap<String, Vec<u8>>);

    impl ReleaseFetcher for MapFetcher {
        fn fetch(&self, url: &str, maximum: u64) -> Result<Vec<u8>, DistributionError> {
            let value = self.0.get(url).ok_or(DistributionError::Fetch)?.clone();
            if u64::try_from(value.len()).map_or(true, |length| length > maximum) {
                return Err(DistributionError::Fetch);
            }
            Ok(value)
        }
    }

    fn build_identity(version: &str, target: &str, source_commit: &str) -> ReleaseBuildIdentity {
        ReleaseBuildIdentity {
            version: version.to_owned(),
            target: target.to_owned(),
            source_commit: source_commit.to_owned(),
            wire_major: zterm_core::WIRE_MAJOR,
            state_schema: zterm_core::STATE_SCHEMA_VERSION,
            release_key_id: RELEASE_KEY_ID.to_owned(),
            classification: ReleaseClassification::Stable,
        }
    }

    fn archive_for(self_check: &ReleaseSelfCheck, marker: &Path) -> Vec<u8> {
        let json = serde_json::to_string(self_check).expect("self-check JSON");
        let script = format!(
            "#!/bin/sh\nset -eu\ncase \"$1\" in\n  --internal-release-self-check) printf '%s\\n' '{json}' ;;\n  --internal-release-verify) exit 0 ;;\n  *) exit 2 ;;\nesac\nprintf touched > '{}'\n",
            marker.display()
        );
        let mut compressed = Vec::new();
        {
            let encoder =
                flate2::write::GzEncoder::new(&mut compressed, flate2::Compression::default());
            let mut archive = tar::Builder::new(encoder);
            let mut header = tar::Header::new_gnu();
            header.set_size(u64::try_from(script.len()).expect("script length"));
            header.set_mode(0o700);
            header.set_mtime(0);
            header.set_cksum();
            archive
                .append_data(&mut header, "zterm", script.as_bytes())
                .expect("candidate archive");
            archive
                .into_inner()
                .expect("gzip encoder")
                .finish()
                .expect("gzip finish");
        }
        compressed
    }

    fn fixture() -> (
        MapFetcher,
        ReleaseSelection,
        [u8; 32],
        BuildIdentity,
        tempfile::TempDir,
    ) {
        let temporary = tempfile::tempdir().expect("fixture root");
        let target = BuildIdentity::current().target;
        assert!(SUPPORTED_RELEASE_TARGETS.contains(&target));
        let version = "9.1.0";
        let tag = "v9.1.0";
        let source_commit = "0123456789abcdef0123456789abcdef01234567";
        let selected_build = build_identity(version, target, source_commit);
        let self_check = ReleaseSelfCheck {
            schema: RELEASE_MANIFEST_SCHEMA,
            product: "zterm".to_owned(),
            build: selected_build.clone(),
        };
        let archive = archive_for(&self_check, &temporary.path().join("candidate-ran"));
        let mut artifacts = Vec::new();
        for item_target in SUPPORTED_RELEASE_TARGETS {
            let filename = artifact_filename(item_target).expect("supported target");
            let selected = item_target == target;
            artifacts.push(ReleaseArtifact {
                filename: filename.clone(),
                target: item_target.to_owned(),
                url: immutable_asset_url(tag, &filename),
                length: if selected {
                    u64::try_from(archive.len()).expect("archive length")
                } else {
                    1
                },
                sha256: if selected {
                    sha256_hex(&archive)
                } else {
                    "11".repeat(32)
                },
                minimum_macos: item_target
                    .ends_with("apple-darwin")
                    .then(|| MINIMUM_MACOS.to_owned()),
                minimum_glibc: item_target
                    .ends_with("unknown-linux-gnu")
                    .then(|| MINIMUM_GLIBC.to_owned()),
                build: if selected {
                    selected_build.clone()
                } else {
                    build_identity(version, item_target, source_commit)
                },
            });
        }
        let manifest = ReleaseManifest {
            schema: RELEASE_MANIFEST_SCHEMA,
            product: "zterm".to_owned(),
            version: version.to_owned(),
            tag: tag.to_owned(),
            classification: ReleaseClassification::Stable,
            source_commit: source_commit.to_owned(),
            released_at: "2026-08-24T12:00:00Z".to_owned(),
            wire_major: zterm_core::WIRE_MAJOR,
            state_schema: zterm_core::STATE_SCHEMA_VERSION,
            bootstrap_schema: RELEASE_BOOTSTRAP_SCHEMA,
            public_key_id: RELEASE_KEY_ID.to_owned(),
            artifacts,
        };
        let pair = Ed25519KeyPair::from_seed_unchecked(&[8; 32]).expect("test key");
        let raw = serde_json::to_vec(&manifest).expect("manifest JSON");
        let signature = pair.sign(&raw).as_ref().to_vec();
        let public_key = pair
            .public_key()
            .as_ref()
            .try_into()
            .expect("public key length");
        let selection = ReleaseSelection::Exact(tag.to_owned());
        let mut values = BTreeMap::new();
        values.insert(selection.asset_url(MANIFEST_ASSET), raw);
        values.insert(selection.asset_url(SIGNATURE_ASSET), signature);
        values.insert(
            manifest
                .artifact_for_target(target)
                .expect("selected artifact")
                .url
                .clone(),
            archive,
        );
        let current = BuildIdentity {
            version: "9.0.0",
            phase: "test",
            target,
            source_commit: "development",
            wire_major: zterm_core::WIRE_MAJOR,
            state_schema: zterm_core::STATE_SCHEMA_VERSION,
            release_key_id: RELEASE_KEY_ID,
            release_classification: "stable",
        };
        (
            MapFetcher(values),
            selection,
            public_key,
            current,
            temporary,
        )
    }

    #[test]
    fn authenticated_candidate_runs_only_after_manifest_and_archive_match() {
        let (fetcher, selection, public_key, current, temporary) = fixture();

        let prepared =
            prepare_with(&fetcher, &selection, &public_key, current).expect("prepared release");

        assert_eq!(prepared.version(), "9.1.0");
        assert_eq!(prepared.target(), BuildIdentity::current().target);
        assert!(temporary.path().join("candidate-ran").exists());
    }

    #[test]
    fn bad_archive_digest_is_rejected_before_candidate_execution() {
        let (mut fetcher, selection, public_key, current, temporary) = fixture();
        let archive_url = fetcher
            .0
            .keys()
            .find(|url| url.ends_with(".tar.gz"))
            .expect("archive URL")
            .clone();
        fetcher
            .0
            .get_mut(&archive_url)
            .expect("archive bytes")
            .push(0);

        assert!(matches!(
            prepare_with(&fetcher, &selection, &public_key, current),
            Err(DistributionError::Fetch | DistributionError::Artifact)
        ));
        assert!(!temporary.path().join("candidate-ran").exists());
    }

    #[test]
    fn exact_selection_rejects_branches_urls_and_noncanonical_tags() {
        for invalid in ["0.1.2", "main", "https://example.com/v0.1.2", "v01.2.3"] {
            assert_eq!(
                ReleaseSelection::parse(Some(invalid)),
                Err(DistributionError::InvalidSelection)
            );
        }
        assert_eq!(
            ReleaseSelection::parse(None),
            Ok(ReleaseSelection::LatestStable)
        );
        assert_eq!(
            ReleaseSelection::parse(Some("v0.2.0-dev.1")),
            Ok(ReleaseSelection::Exact("v0.2.0-dev.1".to_owned()))
        );
    }
}
