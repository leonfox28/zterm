//! Versioned daemon configuration and infrastructure profile validation.

use std::fs;

use iroh::RelayUrl;
use serde::{Deserialize, Deserializer, Serialize};
use zterm_core::DomainErrorKind;
use zterm_platform::user_state::{UserPaths, atomic_write, validate_regular_file};

use crate::error::DaemonError;

/// Current configuration schema.
pub const CONFIG_SCHEMA_VERSION: u32 = 1;

/// Serializable configuration schema version one.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigV1 {
    /// Configuration schema owner.
    pub schema_version: u32,
    /// User-facing device name.
    pub device_name: String,
    /// Mutually exclusive infrastructure selection.
    pub infrastructure: InfrastructureConfig,
}

/// Serializable infrastructure selection; mixed maps are unrepresentable.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "profile")]
pub enum InfrastructureConfig {
    /// Iroh's pinned official n0 production map.
    OfficialN0,
    /// One explicit Relay-only deployment with QAD disabled.
    SelfHosted {
        /// HTTPS Iroh Relay URL.
        relay_url: String,
    },
}

impl<'de> Deserialize<'de> for InfrastructureConfig {
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct WireConfig {
            profile: String,
            relay_url: Option<String>,
        }

        let wire = WireConfig::deserialize(deserializer)?;
        match (wire.profile.as_str(), wire.relay_url) {
            ("official-n0", None) => Ok(Self::OfficialN0),
            ("official-n0", Some(_)) => Err(serde::de::Error::custom(
                "official-n0 must not contain relay_url",
            )),
            ("self-hosted", Some(relay_url)) => Ok(Self::SelfHosted { relay_url }),
            ("self-hosted", None) => Err(serde::de::Error::missing_field("relay_url")),
            (profile, _) => Err(serde::de::Error::unknown_variant(
                profile,
                &["official-n0", "self-hosted"],
            )),
        }
    }
}

/// Infrastructure selection after semantic validation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValidatedInfrastructure {
    /// Official n0 production map.
    OfficialN0,
    /// One explicit self-hosted Relay URL.
    SelfHosted(RelayUrl),
}

impl ValidatedInfrastructure {
    /// Stable config/diagnostic profile name.
    #[must_use]
    pub const fn profile_name(&self) -> &'static str {
        match self {
            Self::OfficialN0 => "official-n0",
            Self::SelfHosted(_) => "self-hosted",
        }
    }

    /// Optional self-hosted URL.
    #[must_use]
    pub fn relay_url(&self) -> Option<&RelayUrl> {
        match self {
            Self::OfficialN0 => None,
            Self::SelfHosted(url) => Some(url),
        }
    }
}

/// Configuration after syntax, schema, name, and profile validation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedConfig {
    /// Trimmed user-facing device name.
    pub device_name: String,
    /// Validated mutually exclusive infrastructure choice.
    pub infrastructure: ValidatedInfrastructure,
}

impl ConfigV1 {
    /// Creates a serializable v1 configuration from validated values.
    #[must_use]
    pub fn from_validated(config: &ValidatedConfig) -> Self {
        let infrastructure = match &config.infrastructure {
            ValidatedInfrastructure::OfficialN0 => InfrastructureConfig::OfficialN0,
            ValidatedInfrastructure::SelfHosted(url) => InfrastructureConfig::SelfHosted {
                relay_url: url.to_string(),
            },
        };
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            device_name: config.device_name.clone(),
            infrastructure,
        }
    }

    /// Applies zterm's semantic configuration validation.
    pub fn validate(self) -> Result<ValidatedConfig, DaemonError> {
        if self.schema_version != CONFIG_SCHEMA_VERSION {
            return Err(DaemonError::new(
                DomainErrorKind::ConfigVersion,
                format!(
                    "unsupported config schema {}, expected {}",
                    self.schema_version, CONFIG_SCHEMA_VERSION
                ),
            ));
        }
        validate_device_name(&self.device_name)?;
        let device_name = self.device_name.trim().to_owned();
        let infrastructure = match self.infrastructure {
            InfrastructureConfig::OfficialN0 => ValidatedInfrastructure::OfficialN0,
            InfrastructureConfig::SelfHosted { relay_url } => {
                let relay_url: RelayUrl = relay_url.parse().map_err(|error| {
                    DaemonError::new(
                        DomainErrorKind::ConfigProfile,
                        format!("invalid self-hosted Relay URL: {error}"),
                    )
                })?;
                if relay_url.scheme() != "https" {
                    return Err(DaemonError::new(
                        DomainErrorKind::ConfigProfile,
                        "self-hosted Relay URL must use https",
                    ));
                }
                ValidatedInfrastructure::SelfHosted(relay_url)
            }
        };
        Ok(ValidatedConfig {
            device_name,
            infrastructure,
        })
    }
}

/// Constructs validated setup input before any persistent mutation.
pub fn validate_setup_input(
    device_name: &str,
    infrastructure: ValidatedInfrastructure,
) -> Result<ValidatedConfig, DaemonError> {
    validate_device_name(device_name)?;
    Ok(ValidatedConfig {
        device_name: device_name.trim().to_owned(),
        infrastructure,
    })
}

/// Validates CLI/profile strings through the one configuration parser.
pub fn validate_setup_profile(
    device_name: &str,
    profile: &str,
    relay_url: Option<&str>,
) -> Result<ValidatedConfig, DaemonError> {
    let infrastructure = match (profile, relay_url) {
        ("official-n0", None) => InfrastructureConfig::OfficialN0,
        ("official-n0", Some(_)) => {
            return Err(DaemonError::new(
                DomainErrorKind::ConfigProfile,
                "official-n0 must not include --relay-url",
            ));
        }
        ("self-hosted", Some(relay_url)) => InfrastructureConfig::SelfHosted {
            relay_url: relay_url.to_owned(),
        },
        ("self-hosted", None) => {
            return Err(DaemonError::new(
                DomainErrorKind::ConfigProfile,
                "self-hosted requires --relay-url <https-url>",
            ));
        }
        (unknown, _) => {
            return Err(DaemonError::new(
                DomainErrorKind::ConfigProfile,
                format!("unknown infrastructure profile {unknown:?}"),
            ));
        }
    };
    ConfigV1 {
        schema_version: CONFIG_SCHEMA_VERSION,
        device_name: device_name.to_owned(),
        infrastructure,
    }
    .validate()
}

/// Loads and validates the committed configuration.
pub fn load_config(paths: &UserPaths) -> Result<ValidatedConfig, DaemonError> {
    validate_regular_file(paths.config(), paths.uid())
        .map_err(|error| DaemonError::new(DomainErrorKind::PathUnsafe, error.to_string()))?;
    let bytes = fs::read(paths.config())
        .map_err(|error| DaemonError::new(DomainErrorKind::ConfigSyntax, error.to_string()))?;
    let config: ConfigV1 = toml::from_slice(&bytes)
        .map_err(|error| DaemonError::new(DomainErrorKind::ConfigSyntax, error.to_string()))?;
    config.validate()
}

/// Atomically writes a validated configuration.
pub fn write_config(paths: &UserPaths, config: &ValidatedConfig) -> Result<(), DaemonError> {
    let bytes = toml::to_string_pretty(&ConfigV1::from_validated(config))
        .map_err(|error| DaemonError::new(DomainErrorKind::ConfigSyntax, error.to_string()))?;
    atomic_write(paths.config(), paths.uid(), |file| {
        use std::io::Write;
        file.write_all(bytes.as_bytes())
    })
    .map_err(|error| DaemonError::new(DomainErrorKind::PathUnsafe, error.to_string()))
}

fn validate_device_name(name: &str) -> Result<(), DaemonError> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.len() > 128 || trimmed.chars().any(char::is_control) {
        Err(DaemonError::new(
            DomainErrorKind::ConfigProfile,
            "device name must contain 1-128 UTF-8 bytes and no control characters",
        ))
    } else {
        Ok(())
    }
}
