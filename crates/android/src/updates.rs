//! Narrow Android adapter for the shared release trust owner.

use crate::NativeError;
use std::fs::File;
use std::sync::Arc;
use zterm_core::release::android::{AndroidRelease, AndroidReleaseFiles, CERTIFICATE, PACKAGE};
use zterm_core::release::{official_release_public_key, require_official_distribution_build};

fn invalid() -> NativeError {
    NativeError::RequestFailed {
        code: "update_invalid".to_owned(),
    }
}

/// Authenticated presentation fields, never accepted back as candidate authority.
#[derive(uniffi::Record)]
pub struct NativeUpdateInfo {
    /// Offered version.
    pub version: String,
    /// Android's monotonic version code.
    pub version_code: u32,
    /// APK byte count.
    pub length: u64,
    /// Immutable download URL.
    pub url: String,
    /// Required application ID.
    pub package: String,
    /// Required APK certificate digest.
    pub certificate_sha256: String,
    /// Minimum Android API.
    pub min_sdk: u32,
}

/// Candidate created only by the shared signed-release verifier.
#[derive(uniffi::Object)]
pub struct NativeUpdate {
    release: AndroidRelease,
}

#[uniffi::export]
impl NativeUpdate {
    /// Returns the verified fields for display and platform checks.
    pub fn info(&self) -> NativeUpdateInfo {
        let m = self.release.metadata();
        NativeUpdateInfo {
            version: m.version.clone(),
            version_code: m.version_code,
            length: m.length,
            url: self.release.apk_url(),
            package: m.package.clone(),
            certificate_sha256: m.certificate_sha256.clone(),
            min_sdk: m.min_sdk,
        }
    }
    /// Requires agreement between SemVer and Android version ordering.
    pub fn is_newer_than(&self, version: String, code: u32) -> Result<bool, NativeError> {
        self.release
            .is_newer_than(&version, code)
            .map_err(|_| invalid())
    }
    /// Hashes a downloaded APK under the signed byte bound.
    pub fn verify_apk(&self, path: String) -> Result<(), NativeError> {
        self.release
            .metadata()
            .verify_apk(File::open(path).map_err(|_| invalid())?)
            .map_err(|_| invalid())
    }
}

/// Rejects development/unofficial installation identities before network discovery.
#[uniffi::export]
pub fn validate_android_update_build(
    package: String,
    certificate: String,
) -> Result<(), NativeError> {
    if package != PACKAGE
        || certificate != CERTIFICATE.trim()
        || require_official_distribution_build(&zterm_core::BuildIdentity::current()).is_err()
    {
        return Err(NativeError::RequestFailed {
            code: "update_unsupported".to_owned(),
        });
    }
    Ok(())
}

/// Checks a discovery tag before the Android HTTP adapter constructs asset URLs.
#[uniffi::export]
pub fn validate_android_update_tag(tag: String) -> Result<(), NativeError> {
    zterm_core::release::android::validate_stable_tag(&tag).map_err(|_| invalid())
}

/// Verifies exact release bytes and returns one opaque candidate.
#[uniffi::export]
pub fn verify_android_update(
    tag: String,
    manifest: Vec<u8>,
    manifest_signature: Vec<u8>,
    checksums: Vec<u8>,
    checksums_signature: Vec<u8>,
    metadata: Vec<u8>,
) -> Result<Arc<NativeUpdate>, NativeError> {
    let release = AndroidRelease::verify(
        &tag,
        AndroidReleaseFiles {
            manifest: &manifest,
            manifest_signature: &manifest_signature,
            checksums: &checksums,
            checksums_signature: &checksums_signature,
            metadata: &metadata,
        },
        &official_release_public_key().map_err(|_| invalid())?,
    )
    .map_err(|_| invalid())?;
    Ok(Arc::new(NativeUpdate { release }))
}
