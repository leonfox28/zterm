//! Android metadata bound to the existing signed release and checksum inventory.

use std::collections::BTreeMap;
use std::io::Read;

use semver::Version;
use serde::{Deserialize, Serialize};

use super::{
    MAX_RELEASE_ARTIFACT_BYTES, MAX_RELEASE_MANIFEST_BYTES, ReleaseClassification, ReleaseError,
    ReleaseManifest, immutable_asset_url, is_lower_hex, sha256_hex, sha256_reader,
    verify_checksums_signature, verify_release_manifest,
};

/// Fixed Android artifact name in the official release inventory.
pub const APK_NAME: &str = "zterm-android-arm64.apk";
/// Fixed Android metadata name in the official release inventory.
pub const METADATA_NAME: &str = "zterm-android.json";
/// Official Android package (development variants cannot replace it).
pub const PACKAGE: &str = "io.github.leonfox28.zterm";
/// Reviewed Android signing certificate digest.
pub const CERTIFICATE: &str = include_str!("../../../../release/android-certificate.sha256");

/// Platform metadata emitted by tools/android/release.py.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AndroidMetadata {
    /// Metadata schema.
    pub schema: u32,
    /// Product name.
    pub product: String,
    /// Canonical Cargo version.
    pub version: String,
    /// Monotonic Android version code.
    pub version_code: u32,
    /// Full release source commit.
    pub source_commit: String,
    /// Android application ID.
    pub package: String,
    /// Packaged native ABI.
    pub abi: String,
    /// Minimum Android API.
    pub min_sdk: u32,
    /// Target Android API.
    pub target_sdk: u32,
    /// APK signing certificate SHA-256.
    pub certificate_sha256: String,
    /// Whether the publisher signed the APK.
    pub signed: bool,
    /// Exact APK byte length.
    pub length: u64,
    /// Exact APK SHA-256.
    pub sha256: String,
}

impl AndroidMetadata {
    /// Validates metadata relationships; authentication is the caller's responsibility.
    pub fn parse(
        raw: &[u8],
        manifest: &ReleaseManifest,
        require_signed: bool,
    ) -> Result<Self, ReleaseError> {
        if raw.is_empty() || raw.len() > MAX_RELEASE_MANIFEST_BYTES {
            return Err(ReleaseError::ManifestSize);
        }
        let value: Self = serde_json::from_slice(raw).map_err(|_| ReleaseError::ManifestSyntax)?;
        if value.schema != 1
            || value.product != "zterm"
            || value.version != manifest.version
            || value.source_commit != manifest.source_commit
            || value.package != PACKAGE
            || value.abi != "arm64-v8a"
            || value.min_sdk < 26
            || value.target_sdk < value.min_sdk
            || value.target_sdk > i32::MAX as u32
            || value.certificate_sha256 != CERTIFICATE.trim()
            || !is_lower_hex(&value.certificate_sha256, 64)
            || !(1..=2_100_000_000).contains(&value.version_code)
            || (require_signed && !value.signed)
        {
            return Err(ReleaseError::BuildIdentityMismatch);
        }
        if value.length == 0 || value.length > MAX_RELEASE_ARTIFACT_BYTES {
            return Err(ReleaseError::ArtifactSize);
        }
        if !is_lower_hex(&value.sha256, 64) {
            return Err(ReleaseError::ArtifactDigest);
        }
        Ok(value)
    }

    /// Checks a bounded APK stream against authenticated metadata.
    pub fn verify_apk(&self, reader: impl Read) -> Result<(), ReleaseError> {
        let (length, digest) = sha256_reader(reader, self.length)?;
        if length != self.length || digest != self.sha256 {
            return Err(ReleaseError::ArtifactDigest);
        }
        Ok(())
    }
}

/// Exact byte owners fetched from one immutable release tag.
pub struct AndroidReleaseFiles<'a> {
    /// Common release manifest.
    pub manifest: &'a [u8],
    /// Detached manifest signature.
    pub manifest_signature: &'a [u8],
    /// Complete checksum inventory.
    pub checksums: &'a [u8],
    /// Detached inventory signature.
    pub checksums_signature: &'a [u8],
    /// Android metadata.
    pub metadata: &'a [u8],
}

/// Authenticated Android candidate; callers cannot construct one from display fields.
#[derive(Debug)]
pub struct AndroidRelease {
    metadata: AndroidMetadata,
    tag: String,
}

impl AndroidRelease {
    /// Verifies both signatures and all selected artifact bindings before returning a candidate.
    pub fn verify(
        tag: &str,
        files: AndroidReleaseFiles<'_>,
        public_key: &[u8; 32],
    ) -> Result<Self, ReleaseError> {
        validate_stable_tag(tag)?;
        let manifest =
            verify_release_manifest(files.manifest, files.manifest_signature, public_key)?;
        if manifest.tag != tag || manifest.classification != ReleaseClassification::Stable {
            return Err(ReleaseError::ReleaseIdentity);
        }
        verify_checksums_signature(files.checksums, files.checksums_signature, public_key)?;
        let inventory = parse_inventory(files.checksums)?;
        require_digest(
            &inventory,
            "zterm-release.json",
            &sha256_hex(files.manifest),
        )?;
        // Bound before hashing or parsing even when called without the HTTP reader.
        if files.metadata.len() > MAX_RELEASE_MANIFEST_BYTES {
            return Err(ReleaseError::ManifestSize);
        }
        require_digest(&inventory, METADATA_NAME, &sha256_hex(files.metadata))?;
        let metadata = AndroidMetadata::parse(files.metadata, &manifest, true)?;
        require_digest(&inventory, APK_NAME, &metadata.sha256)?;
        Ok(Self {
            metadata,
            tag: tag.to_owned(),
        })
    }

    /// Authenticated metadata for presentation and platform compatibility checks.
    #[must_use]
    pub const fn metadata(&self) -> &AndroidMetadata {
        &self.metadata
    }

    /// Fixed-origin immutable download location.
    #[must_use]
    pub fn apk_url(&self) -> String {
        immutable_asset_url(&self.tag, APK_NAME)
    }

    /// Returns whether both independent version orderings permit an update.
    pub fn is_newer_than(&self, version: &str, code: u32) -> Result<bool, ReleaseError> {
        let current = Version::parse(version).map_err(|_| ReleaseError::InvalidVersion)?;
        let candidate =
            Version::parse(&self.metadata.version).map_err(|_| ReleaseError::InvalidVersion)?;
        if current.to_string() != version || !current.build.is_empty() || code == 0 {
            return Err(ReleaseError::InvalidVersion);
        }
        let semantic = candidate.cmp(&current);
        let android = self.metadata.version_code.cmp(&code);
        if semantic != android {
            return Err(ReleaseError::BuildIdentityMismatch);
        }
        Ok(semantic.is_gt())
    }
}

/// Validates an untrusted discovery tag before putting it into an asset URL.
pub fn validate_stable_tag(tag: &str) -> Result<(), ReleaseError> {
    if tag.len() > 128 {
        return Err(ReleaseError::InvalidVersion);
    }
    let text = tag.strip_prefix('v').ok_or(ReleaseError::InvalidVersion)?;
    let version = Version::parse(text).map_err(|_| ReleaseError::InvalidVersion)?;
    if version.to_string() != text || !version.pre.is_empty() || !version.build.is_empty() {
        return Err(ReleaseError::InvalidVersion);
    }
    Ok(())
}

fn parse_inventory(raw: &[u8]) -> Result<BTreeMap<&str, &str>, ReleaseError> {
    let text = std::str::from_utf8(raw).map_err(|_| ReleaseError::ArtifactInventory)?;
    if !text.ends_with('\n') {
        return Err(ReleaseError::ArtifactInventory);
    }
    let mut entries = BTreeMap::new();
    for line in text.split_terminator('\n') {
        let (digest, name) = line
            .split_once("  ")
            .ok_or(ReleaseError::ArtifactInventory)?;
        if !is_lower_hex(digest, 64)
            || name.is_empty()
            || name.len() > 128
            || name.starts_with('.')
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            || entries.insert(name, digest).is_some()
        {
            return Err(ReleaseError::ArtifactInventory);
        }
    }
    Ok(entries)
}

fn require_digest(
    entries: &BTreeMap<&str, &str>,
    name: &str,
    digest: &str,
) -> Result<(), ReleaseError> {
    if entries.get(name).copied() != Some(digest) {
        return Err(ReleaseError::ArtifactDigest);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{Ed25519KeyPair, KeyPair};
    use serde_json::json;

    struct Fixture {
        key: Ed25519KeyPair,
        manifest: Vec<u8>,
        metadata: Vec<u8>,
        apk: Vec<u8>,
    }
    impl Fixture {
        fn new() -> Self {
            let apk = b"test-only APK bytes".to_vec();
            let manifest = serde_json::to_vec(&json!({
                "schema":1,"product":"zterm","version":"0.1.35","tag":"v0.1.35",
                "classification":"stable","source_commit":"a".repeat(40),
                "released_at":"2026-09-20T00:00:00Z","wire_major":2,"state_schema":1,
                "bootstrap_schema":1,"public_key_id":super::super::RELEASE_KEY_ID,"artifacts":[]
            }))
            .expect("valid test fixture");
            let metadata = serde_json::to_vec(&json!({
                "schema":1,"product":"zterm","version":"0.1.35","version_code":103599,
                "source_commit":"a".repeat(40),"package":PACKAGE,"abi":"arm64-v8a",
                "min_sdk":26,"target_sdk":36,"certificate_sha256":CERTIFICATE.trim(),
                "signed":true,"length":apk.len(),"sha256":sha256_hex(&apk)
            }))
            .expect("valid test fixture");
            Self {
                key: Ed25519KeyPair::from_seed_unchecked(&[7; 32]).expect("valid test fixture"),
                manifest,
                metadata,
                apk,
            }
        }
        fn inventory(&self) -> Vec<u8> {
            format!(
                "{}  zterm-release.json\n{}  {METADATA_NAME}\n{}  {APK_NAME}\n",
                sha256_hex(&self.manifest),
                sha256_hex(&self.metadata),
                sha256_hex(&self.apk)
            )
            .into_bytes()
        }
        fn verify(
            &self,
            inventory: &[u8],
            signature: &[u8],
        ) -> Result<AndroidRelease, ReleaseError> {
            AndroidRelease::verify(
                "v0.1.35",
                AndroidReleaseFiles {
                    manifest: &self.manifest,
                    manifest_signature: self.key.sign(&self.manifest).as_ref(),
                    checksums: inventory,
                    checksums_signature: signature,
                    metadata: &self.metadata,
                },
                self.key
                    .public_key()
                    .as_ref()
                    .try_into()
                    .expect("valid test fixture"),
            )
        }
    }
    #[test]
    fn signed_candidate_binds_versions_and_bounded_apk() {
        let f = Fixture::new();
        let inventory = f.inventory();
        let candidate = f
            .verify(&inventory, f.key.sign(&inventory).as_ref())
            .expect("valid test fixture");
        assert!(
            candidate
                .is_newer_than("0.1.34", 103499)
                .expect("valid test fixture")
        );
        assert!(
            !candidate
                .is_newer_than("0.1.35", 103599)
                .expect("valid test fixture")
        );
        assert!(
            !candidate
                .is_newer_than("0.1.36", 103699)
                .expect("valid test fixture")
        );
        assert!(candidate.is_newer_than("0.1.34", 103599).is_err());
        assert!(candidate.metadata().verify_apk(f.apk.as_slice()).is_ok());
        assert!(
            candidate
                .metadata()
                .verify_apk(b"truncated".as_slice())
                .is_err()
        );
        assert!(
            candidate
                .metadata()
                .verify_apk(vec![0; f.apk.len() + 1].as_slice())
                .is_err()
        );
        assert_eq!(
            candidate.apk_url(),
            "https://github.com/leonfox28/zterm/releases/download/v0.1.35/zterm-android-arm64.apk"
        );
    }
    #[test]
    fn rejects_tampered_missing_and_duplicate_inventory() {
        let f = Fixture::new();
        let raw = f.inventory();
        assert!(f.verify(&raw, &[0; 64]).is_err());
        for inventory in [
            format!(
                "{}{}",
                String::from_utf8(raw.clone()).expect("valid test fixture"),
                String::from_utf8(raw.clone()).expect("valid test fixture")
            )
            .into_bytes(),
            format!("{}  {METADATA_NAME}\n", sha256_hex(&f.metadata)).into_bytes(),
            format!("{}  ../{APK_NAME}\n", sha256_hex(&f.apk)).into_bytes(),
        ] {
            assert!(
                f.verify(&inventory, f.key.sign(&inventory).as_ref())
                    .is_err()
            );
        }
        let mut changed = raw.clone();
        changed[0] = if changed[0] == b'a' { b'b' } else { b'a' };
        assert!(f.verify(&changed, f.key.sign(&raw).as_ref()).is_err());
        // Even a newly signed inventory must bind the actual manifest bytes.
        assert!(f.verify(&changed, f.key.sign(&changed).as_ref()).is_err());
    }
    #[test]
    fn rejects_inconsistent_android_identity_even_when_signed() {
        for (key, value) in [
            ("signed", json!(false)),
            ("package", json!("example.other")),
            ("certificate_sha256", json!("0".repeat(64))),
            ("version", json!("0.1.36")),
            ("source_commit", json!("b".repeat(40))),
            ("abi", json!("x86_64")),
            ("version_code", json!(0)),
            ("length", json!(MAX_RELEASE_ARTIFACT_BYTES + 1)),
        ] {
            let mut f = Fixture::new();
            let mut metadata: serde_json::Value =
                serde_json::from_slice(&f.metadata).expect("valid test fixture");
            metadata[key] = value;
            f.metadata = serde_json::to_vec(&metadata).expect("valid test fixture");
            let raw = f.inventory();
            assert!(f.verify(&raw, f.key.sign(&raw).as_ref()).is_err(), "{key}");
        }
    }
    #[test]
    fn discovery_tag_cannot_escape_the_fixed_release_path() {
        for tag in [
            "../latest",
            "v01.2.3",
            "v1.2.3/other",
            "v1.2.3-rc.1",
            "v1.2.3+build",
            "1.2.3",
        ] {
            assert!(validate_stable_tag(tag).is_err());
        }
        assert!(validate_stable_tag("v1.2.3").is_ok());
    }

    #[test]
    fn runtime_accepts_future_sdk_metadata_for_platform_compatibility_checks() {
        let mut fixture = Fixture::new();
        let mut metadata: serde_json::Value =
            serde_json::from_slice(&fixture.metadata).expect("fixture metadata");
        metadata["min_sdk"] = json!(28);
        metadata["target_sdk"] = json!(37);
        fixture.metadata = serde_json::to_vec(&metadata).expect("serialize future SDK metadata");
        let raw = fixture.inventory();
        let candidate = fixture
            .verify(&raw, fixture.key.sign(&raw).as_ref())
            .expect("future platform metadata");
        assert_eq!(candidate.metadata().min_sdk, 28);
        assert_eq!(candidate.metadata().target_sdk, 37);
    }
}
