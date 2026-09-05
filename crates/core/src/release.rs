//! Signed release-manifest contract shared by installers, updates, and tooling.

use std::collections::BTreeSet;
use std::fmt;
use std::io::{self, Read};

use ring::digest::{Context, SHA256};
use ring::signature::{ED25519, UnparsedPublicKey};
use semver::Version;
use serde::{Deserialize, Serialize};

/// Exact manifest schema supported by this product generation.
pub const RELEASE_MANIFEST_SCHEMA: u32 = 1;
/// Exact generated-installer schema supported by this product generation.
pub const RELEASE_BOOTSTRAP_SCHEMA: u32 = 1;
/// Reviewed identifier for the first long-lived release signing key.
pub const RELEASE_KEY_ID: &str = "zterm-release-ed25519-v1";
/// Maximum accepted exact JSON manifest byte length.
pub const MAX_RELEASE_MANIFEST_BYTES: usize = 64 * 1024;
/// Detached Ed25519 signature length.
pub const RELEASE_SIGNATURE_BYTES: usize = 64;
/// Maximum accepted compressed artifact byte length.
pub const MAX_RELEASE_ARTIFACT_BYTES: u64 = 128 * 1024 * 1024;
/// Minimum supported GNU libc version for native Linux releases.
pub const MINIMUM_GLIBC: &str = "2.28";
/// Minimum supported macOS version declared by release artifacts.
pub const MINIMUM_MACOS: &str = "13.0";
/// Official immutable Release asset origin.
pub const RELEASE_ORIGIN: &str = "https://github.com/leonfox28/zterm/releases";

const OFFICIAL_PUBLIC_KEY_HEX: &str = include_str!("../../../release/public-key.hex");

/// Stable/prerelease channel classification authenticated by the manifest.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReleaseClassification {
    /// A normal SemVer without prerelease identifiers.
    Stable,
    /// An explicitly selected SemVer prerelease.
    Prerelease,
}

impl ReleaseClassification {
    /// Classifies one already parsed SemVer.
    #[must_use]
    pub fn from_version(version: &Version) -> Self {
        if version.pre.is_empty() {
            Self::Stable
        } else {
            Self::Prerelease
        }
    }

    /// Stable machine-readable spelling used by build self-checks.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Prerelease => "prerelease",
        }
    }
}

/// Release identity embedded in each target entry.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseBuildIdentity {
    /// Cargo workspace version.
    pub version: String,
    /// Exact Rust target triple.
    pub target: String,
    /// Exact source Git commit.
    pub source_commit: String,
    /// Product wire major.
    pub wire_major: u32,
    /// Persistent-state schema version.
    pub state_schema: u32,
    /// Reviewed release verification-key identifier.
    pub release_key_id: String,
    /// Stable/prerelease classification.
    pub classification: ReleaseClassification,
}

/// One compressed native binary authenticated by a release manifest.
/// Additional platform-specific fields do not change schema-v1's common fields.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReleaseArtifact {
    /// Fixed GitHub Release asset filename.
    pub filename: String,
    /// Exact Rust target triple.
    pub target: String,
    /// Immutable HTTPS asset URL.
    pub url: String,
    /// Exact compressed byte length.
    pub length: u64,
    /// Lowercase hexadecimal SHA-256 digest of the compressed bytes.
    pub sha256: String,
    /// Minimum macOS version, only for Apple artifacts.
    pub minimum_macos: Option<String>,
    /// Minimum glibc version, only for GNU/Linux artifacts.
    pub minimum_glibc: Option<String>,
    /// Cross-checked identity returned by the contained binary.
    pub build: ReleaseBuildIdentity,
}

/// Exact-byte JSON document authenticated by one detached Ed25519 signature.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseManifest {
    /// Manifest schema major.
    pub schema: u32,
    /// Product name; always `zterm`.
    pub product: String,
    /// Cargo workspace SemVer.
    pub version: String,
    /// Git tag; always `v` plus `version`.
    pub tag: String,
    /// Stable/prerelease classification.
    pub classification: ReleaseClassification,
    /// Exact source Git commit.
    pub source_commit: String,
    /// UTC release timestamp in second-resolution RFC 3339 form.
    pub released_at: String,
    /// Product wire major.
    pub wire_major: u32,
    /// Persistent-state schema version.
    pub state_schema: u32,
    /// Generated installer schema.
    pub bootstrap_schema: u32,
    /// Reviewed release verification-key identifier.
    pub public_key_id: String,
    /// Authenticated target inventory for this release, independent of current publication policy.
    pub artifacts: Vec<ReleaseArtifact>,
}

/// Side-effect-free machine-readable identity returned by a candidate binary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseSelfCheck {
    /// Self-check schema major.
    pub schema: u32,
    /// Product name; always `zterm`.
    pub product: String,
    /// Exact embedded build identity.
    pub build: ReleaseBuildIdentity,
}

impl ReleaseSelfCheck {
    /// Returns the current process's compile-time identity without touching user state.
    #[must_use]
    pub fn current() -> Self {
        let build = crate::BuildIdentity::current();
        let classification = match build.release_classification {
            "stable" => ReleaseClassification::Stable,
            "prerelease" => ReleaseClassification::Prerelease,
            _ => ReleaseClassification::Prerelease,
        };
        Self {
            schema: RELEASE_MANIFEST_SCHEMA,
            product: "zterm".to_owned(),
            build: ReleaseBuildIdentity {
                version: build.version.to_owned(),
                target: build.target.to_owned(),
                source_commit: build.source_commit.to_owned(),
                wire_major: build.wire_major,
                state_schema: build.state_schema,
                release_key_id: build.release_key_id.to_owned(),
                classification,
            },
        }
    }

    /// Cross-checks this self-check against the updater's authenticated selected artifact.
    pub fn require_artifact(&self, artifact: &ReleaseArtifact) -> Result<(), ReleaseError> {
        if self.schema != RELEASE_MANIFEST_SCHEMA
            || self.product != "zterm"
            || artifact.build != self.build
        {
            return Err(ReleaseError::BuildIdentityMismatch);
        }
        Ok(())
    }
}

impl ReleaseManifest {
    /// Selects and validates the unique requested target; other targets are not inspected.
    /// The caller must first authenticate the manifest's exact bytes.
    pub fn artifact_for_target(&self, target: &str) -> Result<&ReleaseArtifact, ReleaseError> {
        let mut matches = self
            .artifacts
            .iter()
            .filter(|artifact| artifact.target == target);
        let artifact = matches.next().ok_or(ReleaseError::UnsupportedTarget)?;
        if matches.next().is_some() {
            return Err(ReleaseError::ArtifactInventory);
        }
        validate_artifact(self, artifact)?;
        Ok(artifact)
    }

    /// Parses the manifest's validated SemVer.
    pub fn parsed_version(&self) -> Result<Version, ReleaseError> {
        Version::parse(&self.version).map_err(|_| ReleaseError::InvalidVersion)
    }

    /// Rejects same-version updates and managed downgrades.
    pub fn require_newer_than(&self, current: &str) -> Result<(), ReleaseError> {
        let current = Version::parse(current).map_err(|_| ReleaseError::InvalidVersion)?;
        let candidate = self.parsed_version()?;
        if candidate <= current {
            return Err(ReleaseError::NotNewer);
        }
        Ok(())
    }

    /// Cross-checks one release candidate's embedded identity.
    pub fn require_build_identity(
        &self,
        build: &crate::BuildIdentity,
    ) -> Result<&ReleaseArtifact, ReleaseError> {
        let artifact = self.artifact_for_target(build.target)?;
        let classification = classification_from_build(build.release_classification)?;
        if build.version != self.version
            || build.source_commit != self.source_commit
            || build.wire_major != self.wire_major
            || build.state_schema != self.state_schema
            || build.release_key_id != self.public_key_id
            || classification != self.classification
            || artifact.build.version != build.version
            || artifact.build.target != build.target
            || artifact.build.source_commit != build.source_commit
            || artifact.build.wire_major != build.wire_major
            || artifact.build.state_schema != build.state_schema
            || artifact.build.release_key_id != build.release_key_id
            || artifact.build.classification != classification
        {
            return Err(ReleaseError::BuildIdentityMismatch);
        }
        Ok(artifact)
    }
}

/// Stable content-free failure from the release trust boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReleaseError {
    /// Manifest bytes were empty or crossed the fixed size bound.
    ManifestSize,
    /// Detached signature was not exactly one Ed25519 signature.
    SignatureSize,
    /// Production public key has not passed the external configuration checkpoint.
    PublicKeyUnavailable,
    /// Detached signature did not authenticate the exact manifest bytes.
    SignatureInvalid,
    /// JSON syntax, duplicate fields, or an unknown field was rejected.
    ManifestSyntax,
    /// Manifest or bootstrap schema is not supported.
    Schema,
    /// Product, tag, or release classification was inconsistent.
    ReleaseIdentity,
    /// SemVer was invalid.
    InvalidVersion,
    /// Source commit or release timestamp was malformed.
    SourceIdentity,
    /// Public-key identifier did not match the reviewed source.
    KeyIdentifier,
    /// Artifact inventory was incomplete or contained duplicate targets.
    ArtifactInventory,
    /// Artifact filename or URL was not the immutable expected value.
    ArtifactLocation,
    /// Artifact length was zero or exceeded the product bound.
    ArtifactSize,
    /// Artifact SHA-256 was not canonical lowercase hexadecimal.
    ArtifactDigest,
    /// Platform support floor did not match the release contract.
    PlatformFloor,
    /// Requested build target is not in this authenticated release's inventory.
    UnsupportedTarget,
    /// Candidate binary identity did not match authenticated metadata.
    BuildIdentityMismatch,
    /// Candidate is the current version or a managed downgrade.
    NotNewer,
    /// Reading a bounded digest source failed.
    DigestRead,
}

impl fmt::Display for ReleaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ManifestSize => "release manifest size is invalid",
            Self::SignatureSize => "release signature size is invalid",
            Self::PublicKeyUnavailable => "official release public key is not configured",
            Self::SignatureInvalid => "release manifest signature is invalid",
            Self::ManifestSyntax => "release manifest syntax is invalid",
            Self::Schema => "release schema is unsupported",
            Self::ReleaseIdentity => "release tag or classification is inconsistent",
            Self::InvalidVersion => "release version is invalid",
            Self::SourceIdentity => "release source identity is invalid",
            Self::KeyIdentifier => "release public-key identifier is invalid",
            Self::ArtifactInventory => "release artifact inventory is invalid",
            Self::ArtifactLocation => "release artifact location is invalid",
            Self::ArtifactSize => "release artifact size is invalid",
            Self::ArtifactDigest => "release artifact digest is invalid",
            Self::PlatformFloor => "release platform support floor is invalid",
            Self::UnsupportedTarget => "release target is unsupported",
            Self::BuildIdentityMismatch => "release candidate build identity does not match",
            Self::NotNewer => "release candidate is not newer than the installed version",
            Self::DigestRead => "unable to read release artifact for digest verification",
        })
    }
}

impl std::error::Error for ReleaseError {}

/// Authenticates exact bytes and validates schema-v1 release metadata.
/// Select and validate the required platform with `artifact_for_target` before use.
pub fn verify_release_manifest(
    raw_manifest: &[u8],
    signature: &[u8],
    public_key: &[u8; 32],
) -> Result<ReleaseManifest, ReleaseError> {
    if raw_manifest.is_empty() || raw_manifest.len() > MAX_RELEASE_MANIFEST_BYTES {
        return Err(ReleaseError::ManifestSize);
    }
    if signature.len() != RELEASE_SIGNATURE_BYTES {
        return Err(ReleaseError::SignatureSize);
    }
    UnparsedPublicKey::new(&ED25519, public_key)
        .verify(raw_manifest, signature)
        .map_err(|_| ReleaseError::SignatureInvalid)?;
    let manifest: ReleaseManifest =
        serde_json::from_slice(raw_manifest).map_err(|_| ReleaseError::ManifestSyntax)?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

/// Verifies one manifest with the reviewed production public key.
pub fn verify_official_release_manifest(
    raw_manifest: &[u8],
    signature: &[u8],
) -> Result<ReleaseManifest, ReleaseError> {
    let public_key = official_release_public_key()?;
    verify_release_manifest(raw_manifest, signature, &public_key)
}

/// Decodes the reviewed production public key, refusing the explicit placeholder.
pub fn official_release_public_key() -> Result<[u8; 32], ReleaseError> {
    let text = OFFICIAL_PUBLIC_KEY_HEX.trim();
    let key = decode_hex_32(text).ok_or(ReleaseError::PublicKeyUnavailable)?;
    if key.iter().all(|byte| *byte == 0) {
        return Err(ReleaseError::PublicKeyUnavailable);
    }
    Ok(key)
}

/// Proves that a running binary was built for the official managed channel.
///
/// This intentionally does not restrict the installation directory: a user
/// may choose any otherwise-safe owned directory. It rejects development and
/// ordinary CI binaries so `update`/`uninstall` cannot replace or delete a
/// repository build merely because its filesystem ownership looks safe.
pub fn require_official_distribution_build(
    build: &crate::BuildIdentity,
) -> Result<(), ReleaseError> {
    let _ = official_release_public_key()?;
    validate_distribution_build(build)
}

/// Validates every artifact for release authoring, independently of a publication matrix.
/// Runtime consumers authenticate the manifest and validate only their selected artifact.
pub fn validate_unsigned_manifest(manifest: &ReleaseManifest) -> Result<(), ReleaseError> {
    validate_manifest(manifest)?;
    if manifest.artifacts.is_empty() {
        return Err(ReleaseError::ArtifactInventory);
    }
    let mut targets = BTreeSet::new();
    for artifact in &manifest.artifacts {
        if !targets.insert(artifact.target.as_str()) {
            return Err(ReleaseError::ArtifactInventory);
        }
        validate_artifact(manifest, artifact)?;
    }
    Ok(())
}

/// Computes canonical lowercase SHA-256 for in-memory bytes.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    encode_hex(ring::digest::digest(&SHA256, bytes).as_ref())
}

/// Computes canonical lowercase SHA-256 while enforcing an absolute byte bound.
pub fn sha256_reader(mut reader: impl Read, maximum: u64) -> Result<(u64, String), ReleaseError> {
    let mut context = Context::new(&SHA256);
    let mut length = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|_| ReleaseError::DigestRead)?;
        if read == 0 {
            break;
        }
        length = length
            .checked_add(u64::try_from(read).map_err(|_| ReleaseError::ArtifactSize)?)
            .ok_or(ReleaseError::ArtifactSize)?;
        if length > maximum {
            return Err(ReleaseError::ArtifactSize);
        }
        context.update(&buffer[..read]);
    }
    Ok((length, encode_hex(context.finish().as_ref())))
}

/// Returns the fixed archive filename for one path-safe target identifier.
pub fn artifact_filename(target: &str) -> Result<String, ReleaseError> {
    if !is_target_identifier(target) {
        return Err(ReleaseError::UnsupportedTarget);
    }
    Ok(format!("zterm-{target}.tar.gz"))
}

/// Returns the exact immutable GitHub asset URL for one tag and filename.
#[must_use]
pub fn immutable_asset_url(tag: &str, filename: &str) -> String {
    format!("{RELEASE_ORIGIN}/download/{tag}/{filename}")
}

fn validate_manifest(manifest: &ReleaseManifest) -> Result<(), ReleaseError> {
    if manifest.schema != RELEASE_MANIFEST_SCHEMA
        || manifest.bootstrap_schema != RELEASE_BOOTSTRAP_SCHEMA
    {
        return Err(ReleaseError::Schema);
    }
    if manifest.product != "zterm" {
        return Err(ReleaseError::ReleaseIdentity);
    }
    let version = manifest.parsed_version()?;
    if manifest.tag != format!("v{version}")
        || manifest.classification != ReleaseClassification::from_version(&version)
    {
        return Err(ReleaseError::ReleaseIdentity);
    }
    if !is_lower_hex(&manifest.source_commit, 40) || !is_release_timestamp(&manifest.released_at) {
        return Err(ReleaseError::SourceIdentity);
    }
    if manifest.wire_major == 0 || manifest.state_schema == 0 {
        return Err(ReleaseError::ReleaseIdentity);
    }
    if manifest.public_key_id != RELEASE_KEY_ID {
        return Err(ReleaseError::KeyIdentifier);
    }
    Ok(())
}

fn validate_artifact(
    manifest: &ReleaseManifest,
    artifact: &ReleaseArtifact,
) -> Result<(), ReleaseError> {
    let filename = artifact_filename(&artifact.target)?;
    if artifact.filename != filename
        || artifact.url != immutable_asset_url(&manifest.tag, &filename)
    {
        return Err(ReleaseError::ArtifactLocation);
    }
    if artifact.length == 0 || artifact.length > MAX_RELEASE_ARTIFACT_BYTES {
        return Err(ReleaseError::ArtifactSize);
    }
    if !is_lower_hex(&artifact.sha256, 64) {
        return Err(ReleaseError::ArtifactDigest);
    }
    validate_platform_floor(artifact)?;
    if artifact.build.version != manifest.version
        || artifact.build.target != artifact.target
        || artifact.build.source_commit != manifest.source_commit
        || artifact.build.wire_major != manifest.wire_major
        || artifact.build.state_schema != manifest.state_schema
        || artifact.build.release_key_id != manifest.public_key_id
        || artifact.build.classification != manifest.classification
    {
        return Err(ReleaseError::BuildIdentityMismatch);
    }
    Ok(())
}

fn validate_platform_floor(artifact: &ReleaseArtifact) -> Result<(), ReleaseError> {
    if artifact.target.ends_with("apple-darwin") {
        if artifact.minimum_macos.as_deref() != Some(MINIMUM_MACOS)
            || artifact.minimum_glibc.is_some()
        {
            return Err(ReleaseError::PlatformFloor);
        }
    } else if artifact.target.ends_with("unknown-linux-gnu") {
        if artifact.minimum_glibc.as_deref() != Some(MINIMUM_GLIBC)
            || artifact.minimum_macos.is_some()
        {
            return Err(ReleaseError::PlatformFloor);
        }
    } else {
        return Err(ReleaseError::UnsupportedTarget);
    }
    Ok(())
}

fn decode_hex_32(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 {
        return None;
    }
    let mut result = [0_u8; 32];
    let (pairs, remainder) = text.as_bytes().as_chunks::<2>();
    if !remainder.is_empty() {
        return None;
    }
    for (index, pair) in pairs.iter().enumerate() {
        result[index] = hex_value(pair[0])?.checked_mul(16)? + hex_value(pair[1])?;
    }
    Some(result)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn is_lower_hex(text: &str, length: usize) -> bool {
    text.len() == length
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_release_timestamp(text: &str) -> bool {
    if text.len() != 20 {
        return false;
    }
    let bytes = text.as_bytes();
    for index in [4, 7, 10, 13, 16, 19] {
        let expected = match index {
            4 | 7 => b'-',
            10 => b'T',
            13 | 16 => b':',
            19 => b'Z',
            _ => return false,
        };
        if bytes[index] != expected {
            return false;
        }
    }
    bytes
        .iter()
        .enumerate()
        .all(|(index, byte)| [4, 7, 10, 13, 16, 19].contains(&index) || byte.is_ascii_digit())
}

fn classification_from_build(value: &str) -> Result<ReleaseClassification, ReleaseError> {
    match value {
        "stable" => Ok(ReleaseClassification::Stable),
        "prerelease" => Ok(ReleaseClassification::Prerelease),
        _ => Err(ReleaseError::BuildIdentityMismatch),
    }
}

fn validate_distribution_build(build: &crate::BuildIdentity) -> Result<(), ReleaseError> {
    let version = Version::parse(build.version).map_err(|_| ReleaseError::InvalidVersion)?;
    if build.version != version.to_string()
        || build.phase != crate::PHASE_NAME
        || !is_target_identifier(build.target)
        || !is_lower_hex(build.source_commit, 40)
        || build.source_commit.bytes().all(|byte| byte == b'0')
        || build.wire_major != crate::WIRE_MAJOR
        || build.state_schema != crate::STATE_SCHEMA_VERSION
        || build.release_key_id != RELEASE_KEY_ID
        || classification_from_build(build.release_classification)?
            != ReleaseClassification::from_version(&version)
    {
        return Err(ReleaseError::BuildIdentityMismatch);
    }
    Ok(())
}

fn is_target_identifier(target: &str) -> bool {
    !target.is_empty()
        && target
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

impl From<io::Error> for ReleaseError {
    fn from(_: io::Error) -> Self {
        Self::DigestRead
    }
}

#[cfg(test)]
mod tests {
    use ring::signature::{Ed25519KeyPair, KeyPair};

    use super::*;

    const TEST_SEED: [u8; 32] = [7; 32];
    const HISTORICAL_TARGETS: [&str; 4] = [
        "aarch64-apple-darwin",
        "x86_64-apple-darwin",
        "aarch64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu",
    ];

    fn fixture() -> ReleaseManifest {
        let version = "0.1.2".to_owned();
        let tag = "v0.1.2".to_owned();
        let source_commit = "0123456789abcdef0123456789abcdef01234567".to_owned();
        let artifacts = HISTORICAL_TARGETS
            .iter()
            .map(|target| {
                let filename = artifact_filename(target).expect("supported target");
                let apple = target.ends_with("apple-darwin");
                ReleaseArtifact {
                    url: immutable_asset_url(&tag, &filename),
                    filename,
                    target: (*target).to_owned(),
                    length: 42,
                    sha256: "ab".repeat(32),
                    minimum_macos: apple.then(|| MINIMUM_MACOS.to_owned()),
                    minimum_glibc: (!apple).then(|| MINIMUM_GLIBC.to_owned()),
                    build: ReleaseBuildIdentity {
                        version: version.clone(),
                        target: (*target).to_owned(),
                        source_commit: source_commit.clone(),
                        wire_major: crate::WIRE_MAJOR,
                        state_schema: crate::STATE_SCHEMA_VERSION,
                        release_key_id: RELEASE_KEY_ID.to_owned(),
                        classification: ReleaseClassification::Stable,
                    },
                }
            })
            .collect();
        ReleaseManifest {
            schema: RELEASE_MANIFEST_SCHEMA,
            product: "zterm".to_owned(),
            version,
            tag,
            classification: ReleaseClassification::Stable,
            source_commit,
            released_at: "2026-08-24T12:00:00Z".to_owned(),
            wire_major: crate::WIRE_MAJOR,
            state_schema: crate::STATE_SCHEMA_VERSION,
            bootstrap_schema: RELEASE_BOOTSTRAP_SCHEMA,
            public_key_id: RELEASE_KEY_ID.to_owned(),
            artifacts,
        }
    }

    fn signed(manifest: &impl Serialize) -> (Vec<u8>, Vec<u8>, [u8; 32]) {
        let pair = Ed25519KeyPair::from_seed_unchecked(&TEST_SEED).expect("test signing key");
        let raw = serde_json::to_vec(manifest).expect("fixture JSON");
        let signature = pair.sign(&raw).as_ref().to_vec();
        let public_key: [u8; 32] = pair
            .public_key()
            .as_ref()
            .try_into()
            .expect("Ed25519 public key length");
        (raw, signature, public_key)
    }

    #[test]
    fn exact_bytes_and_complete_inventory_verify() {
        let manifest = fixture();
        let (raw, signature, public_key) = signed(&manifest);

        let verified = verify_release_manifest(&raw, &signature, &public_key)
            .expect("valid signed release manifest");

        assert_eq!(verified, manifest);
        assert_eq!(
            verified.artifact_for_target(HISTORICAL_TARGETS[0]),
            Ok(&verified.artifacts[0])
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn only_the_requested_target_must_be_present() {
        let historical = fixture();
        let (raw, signature, public_key) = signed(&historical);
        assert_eq!(
            verify_release_manifest(&raw, &signature, &public_key),
            Ok(historical.clone())
        );

        let mut current = historical;
        current
            .artifacts
            .retain(|artifact| artifact.target == "aarch64-apple-darwin");
        let (raw, signature, public_key) = signed(&current);
        let verified = verify_release_manifest(&raw, &signature, &public_key)
            .expect("single-target signed manifest");
        assert_eq!(verified.artifacts.len(), 1);
        assert_eq!(
            verified.artifact_for_target("x86_64-apple-darwin"),
            Err(ReleaseError::UnsupportedTarget)
        );
        assert!(verified.artifact_for_target("aarch64-apple-darwin").is_ok());

        current.artifacts.clear();
        let (raw, signature, public_key) = signed(&current);
        let verified = verify_release_manifest(&raw, &signature, &public_key)
            .expect("authenticated release metadata");
        assert_eq!(
            verified.artifact_for_target("aarch64-apple-darwin"),
            Err(ReleaseError::UnsupportedTarget)
        );
        assert_eq!(
            validate_unsigned_manifest(&current),
            Err(ReleaseError::ArtifactInventory)
        );
    }

    #[test]
    fn unrelated_future_platforms_do_not_affect_the_selected_artifact() {
        let mut manifest = fixture();
        for index in 0..4 {
            let mut future = manifest.artifacts[0].clone();
            future.target = format!("future-{index}");
            future.filename = "future.zip".to_owned();
            future.url = "https://example.invalid/future.zip".to_owned();
            future.length = 0;
            future.sha256 = "future-digest-format".to_owned();
            future.minimum_macos = None;
            manifest.artifacts.push(future);
        }
        let mut document = serde_json::to_value(&manifest).expect("manifest JSON");
        document["artifacts"][4]["minimum_future_os"] = serde_json::json!("1.0");
        let (raw, signature, public_key) = signed(&document);
        let verified = verify_release_manifest(&raw, &signature, &public_key)
            .expect("platform-independent signed manifest");

        assert_eq!(verified.artifacts.len(), 8);
        assert_eq!(
            verified.artifact_for_target("aarch64-apple-darwin"),
            Ok(&manifest.artifacts[0])
        );
        assert!(validate_unsigned_manifest(&verified).is_err());
    }

    #[test]
    fn selected_artifact_still_requires_its_location_digest_floor_and_identity() {
        let manifest = fixture();
        let mut mismatched_build = manifest.artifacts[0].build.clone();
        mismatched_build.source_commit = "ff".repeat(20);
        for (field, value, expected) in [
            (
                "url",
                serde_json::json!("https://example.invalid/zterm.tar.gz"),
                ReleaseError::ArtifactLocation,
            ),
            (
                "sha256",
                serde_json::json!("invalid-digest"),
                ReleaseError::ArtifactDigest,
            ),
            (
                "minimum_macos",
                serde_json::json!("99.0"),
                ReleaseError::PlatformFloor,
            ),
            (
                "build",
                serde_json::to_value(&mismatched_build).expect("build JSON"),
                ReleaseError::BuildIdentityMismatch,
            ),
        ] {
            let mut document = serde_json::to_value(&manifest).expect("manifest JSON");
            document["artifacts"][0][field] = value;
            let (raw, signature, public_key) = signed(&document);
            let verified = verify_release_manifest(&raw, &signature, &public_key)
                .expect("authenticated release metadata");
            assert_eq!(
                verified.artifact_for_target("aarch64-apple-darwin"),
                Err(expected),
                "selected artifact field {field}"
            );
        }
    }

    #[test]
    fn candidate_self_check_must_match_the_updaters_selected_platform() {
        let manifest = fixture();
        let candidate = ReleaseSelfCheck {
            schema: RELEASE_MANIFEST_SCHEMA,
            product: "zterm".to_owned(),
            build: manifest.artifacts[1].build.clone(),
        };
        assert_eq!(
            candidate.require_artifact(&manifest.artifacts[0]),
            Err(ReleaseError::BuildIdentityMismatch)
        );
        assert_eq!(candidate.require_artifact(&manifest.artifacts[1]), Ok(()));
    }

    #[test]
    fn signature_is_checked_before_json_and_exact_bytes_cannot_change() {
        let manifest = fixture();
        let (mut raw, signature, public_key) = signed(&manifest);
        raw.push(b'\n');

        assert_eq!(
            verify_release_manifest(&raw, &signature, &public_key),
            Err(ReleaseError::SignatureInvalid)
        );
        assert_eq!(
            verify_release_manifest(b"not-json", &[0; RELEASE_SIGNATURE_BYTES], &public_key),
            Err(ReleaseError::SignatureInvalid)
        );
        assert_eq!(
            verify_release_manifest(b"not-json", &[0; RELEASE_SIGNATURE_BYTES - 1], &public_key),
            Err(ReleaseError::SignatureSize)
        );
    }

    #[test]
    fn explicit_prerelease_verifies_and_invalid_version_is_rejected() {
        let mut prerelease = fixture();
        prerelease.version = "0.2.0-rc.1".to_owned();
        prerelease.tag = "v0.2.0-rc.1".to_owned();
        prerelease.classification = ReleaseClassification::Prerelease;
        for artifact in &mut prerelease.artifacts {
            artifact.url = immutable_asset_url(&prerelease.tag, &artifact.filename);
            artifact.build.version.clone_from(&prerelease.version);
            artifact.build.classification = ReleaseClassification::Prerelease;
        }
        let (raw, signature, public_key) = signed(&prerelease);
        assert_eq!(
            verify_release_manifest(&raw, &signature, &public_key),
            Ok(prerelease)
        );

        let mut invalid_version = fixture();
        invalid_version.version = "not-semver".to_owned();
        let (raw, signature, public_key) = signed(&invalid_version);
        assert_eq!(
            verify_release_manifest(&raw, &signature, &public_key),
            Err(ReleaseError::InvalidVersion)
        );
    }

    #[test]
    fn schema_duplicate_target_size_and_classification_cross_real_boundaries() {
        let mut manifest = fixture();
        manifest.schema += 1;
        let (raw, signature, public_key) = signed(&manifest);
        assert_eq!(
            verify_release_manifest(&raw, &signature, &public_key),
            Err(ReleaseError::Schema)
        );

        let mut manifest = fixture();
        manifest.artifacts[1].target = manifest.artifacts[0].target.clone();
        let (raw, signature, public_key) = signed(&manifest);
        assert_eq!(
            verify_release_manifest(&raw, &signature, &public_key)
                .expect("authenticated metadata")
                .artifact_for_target("aarch64-apple-darwin"),
            Err(ReleaseError::ArtifactInventory)
        );

        let mut manifest = fixture();
        manifest.artifacts[0].length = MAX_RELEASE_ARTIFACT_BYTES + 1;
        let (raw, signature, public_key) = signed(&manifest);
        assert_eq!(
            verify_release_manifest(&raw, &signature, &public_key)
                .expect("authenticated metadata")
                .artifact_for_target("aarch64-apple-darwin"),
            Err(ReleaseError::ArtifactSize)
        );

        let mut manifest = fixture();
        manifest.version = "0.2.0-dev.1".to_owned();
        manifest.tag = "v0.2.0-dev.1".to_owned();
        let (raw, signature, public_key) = signed(&manifest);
        assert_eq!(
            verify_release_manifest(&raw, &signature, &public_key),
            Err(ReleaseError::ReleaseIdentity)
        );
    }

    #[test]
    fn manifest_bounds_and_update_monotonicity_are_explicit() {
        assert_eq!(
            verify_release_manifest(&[], &[], &[1; 32]),
            Err(ReleaseError::ManifestSize)
        );
        assert_eq!(
            verify_release_manifest(
                &vec![b' '; MAX_RELEASE_MANIFEST_BYTES + 1],
                &[0; RELEASE_SIGNATURE_BYTES],
                &[1; 32]
            ),
            Err(ReleaseError::ManifestSize)
        );
        let manifest = fixture();
        assert_eq!(manifest.require_newer_than("0.1.1"), Ok(()));
        assert_eq!(
            manifest.require_newer_than("0.1.2"),
            Err(ReleaseError::NotNewer)
        );
        assert_eq!(
            manifest.require_newer_than("0.2.0"),
            Err(ReleaseError::NotNewer)
        );
    }

    #[test]
    fn bounded_reader_rejects_truncation_growth_before_unbounded_allocation() {
        let bytes = vec![3_u8; 1025];
        assert_eq!(
            sha256_reader(bytes.as_slice(), 1024),
            Err(ReleaseError::ArtifactSize)
        );
        let (length, digest) = sha256_reader(b"abc".as_slice(), 3).expect("bounded digest");
        assert_eq!(length, 3);
        assert_eq!(digest, sha256_hex(b"abc"));
    }

    #[test]
    fn managed_distribution_proof_rejects_development_and_ambient_ci_builds() {
        let development = crate::BuildIdentity::current();
        assert_eq!(
            validate_distribution_build(&development),
            Err(ReleaseError::BuildIdentityMismatch)
        );

        let official = crate::BuildIdentity {
            version: "0.1.2",
            phase: crate::PHASE_NAME,
            target: "aarch64-apple-darwin",
            source_commit: "0123456789abcdef0123456789abcdef01234567",
            wire_major: crate::WIRE_MAJOR,
            state_schema: crate::STATE_SCHEMA_VERSION,
            release_key_id: RELEASE_KEY_ID,
            release_classification: "stable",
        };
        assert_eq!(validate_distribution_build(&official), Ok(()));

        let ordinary_ci = crate::BuildIdentity {
            source_commit: "development",
            ..official
        };
        assert_eq!(
            validate_distribution_build(&ordinary_ci),
            Err(ReleaseError::BuildIdentityMismatch)
        );
    }
}
