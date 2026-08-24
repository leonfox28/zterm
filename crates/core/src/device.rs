//! Transport-neutral device display and relay address values.

use std::fmt;

use crate::{AuthGeneration, AuthorizationStatus, DeviceId};

/// Maximum encoded UTF-8 bytes in a device alias or display name.
pub const MAX_DEVICE_ALIAS_BYTES: usize = 128;
/// Maximum encoded UTF-8 bytes in a remote-provided device display name.
pub const MAX_DEVICE_DISPLAY_NAME_BYTES: usize = 128;
/// Reserved alias value which selects the local device and is never assignable.
pub const RESERVED_DEVICE_ALIAS: &str = "local";
/// Maximum bytes in one relay hint URL.
pub const MAX_RELAY_URL_BYTES: usize = 2048;

/// Number of EndpointId bytes retained in a disambiguating alias suffix.
const ALIAS_SUFFIX_ID_BYTES: usize = 4;

/// Validated remote-provided device display name.
///
/// This is the shared boundary for ticket, connection-handshake, device
/// projection, and persisted known-device names. It deliberately does not
/// apply alias-only policy such as the reserved `local` value or surrounding
/// whitespace; callers choose a local [`DeviceAlias`] separately.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DeviceDisplayName(String);

impl DeviceDisplayName {
    /// Validates a display name without trimming or normalizing it.
    pub fn new(value: impl Into<String>) -> Result<Self, DeviceDisplayNameError> {
        let value = value.into();
        if value.is_empty() {
            return Err(DeviceDisplayNameError::Empty);
        }
        if value.len() > MAX_DEVICE_DISPLAY_NAME_BYTES {
            return Err(DeviceDisplayNameError::TooLong {
                actual: value.len(),
                maximum: MAX_DEVICE_DISPLAY_NAME_BYTES,
            });
        }
        if value.chars().any(char::is_control) {
            return Err(DeviceDisplayNameError::ControlCharacter);
        }
        Ok(Self(value))
    }

    /// Borrows the validated display name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the owned validated string.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for DeviceDisplayName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl AsRef<str> for DeviceDisplayName {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::ops::Deref for DeviceDisplayName {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

/// Failure while validating a device display name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceDisplayNameError {
    /// A display name must contain at least one byte.
    Empty,
    /// A display name exceeded its UTF-8 byte ceiling.
    TooLong {
        /// Observed byte count.
        actual: usize,
        /// Maximum accepted byte count.
        maximum: usize,
    },
    /// Unicode control characters are forbidden.
    ControlCharacter,
}

impl fmt::Display for DeviceDisplayNameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(formatter, "device display name must not be empty"),
            Self::TooLong { actual, maximum } => write!(
                formatter,
                "device display name must contain at most {maximum} UTF-8 bytes, got {actual}"
            ),
            Self::ControlCharacter => {
                write!(
                    formatter,
                    "device display name must not contain control characters"
                )
            }
        }
    }
}

impl std::error::Error for DeviceDisplayNameError {}

/// Validated, exact local alias for one known remote device.
///
/// Aliases are unique across the local address book and may not claim the
/// reserved [`RESERVED_DEVICE_ALIAS`] value. Uniqueness itself is enforced by
/// the owning [`crate::DeviceAlias`] directory and its SQLite unique index; this
/// value only owns the syntactic contract.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DeviceAlias(String);

impl DeviceAlias {
    /// Validates an explicit alias without trimming or normalizing it.
    pub fn new(value: impl Into<String>) -> Result<Self, DeviceAliasError> {
        let value = value.into();
        validate_alias(&value)?;
        Ok(Self(value))
    }

    /// Preferred default alias from a remote-provided display name.
    ///
    /// Returns `None` when the name is empty, too long, contains control
    /// characters or surrounding whitespace, or claims the reserved value; the
    /// caller then falls back to [`Self::disambiguated`].
    #[must_use]
    pub fn from_remote_name(remote_name: &str) -> Option<Self> {
        validate_alias(remote_name).ok()?;
        Some(Self(remote_name.to_owned()))
    }

    /// Deterministic fallback alias that appends a short EndpointId suffix.
    ///
    /// The remote name is trimmed of surrounding whitespace, control characters
    /// are dropped, and the remainder is truncated as a prefix so the result
    /// always fits the 128-byte bound. The suffix guarantees the value is
    /// non-empty and distinct from any bare remote name, so this never fails
    /// and always yields a syntactically valid alias.
    #[must_use]
    pub fn disambiguated(remote_name: &str, device_id: &DeviceId) -> Self {
        let suffix = short_id_suffix(device_id);
        let budget = MAX_DEVICE_ALIAS_BYTES.saturating_sub(suffix.len());
        let mut prefix = String::new();
        for character in remote_name.trim().chars() {
            if character.is_control() {
                break;
            }
            if prefix.len() + character.len_utf8() > budget {
                break;
            }
            prefix.push(character);
        }
        let alias = format!("{prefix}{suffix}");
        debug_assert!(alias.len() <= MAX_DEVICE_ALIAS_BYTES);
        debug_assert!(
            validate_alias(&alias).is_ok(),
            "disambiguated alias must be valid"
        );
        Self(alias)
    }

    /// Borrows the validated alias.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Deterministic short EndpointId suffix used to disambiguate default aliases.
#[must_use]
pub fn short_endpoint_id(device_id: &DeviceId) -> String {
    let mut output = String::with_capacity(ALIAS_SUFFIX_ID_BYTES * 2);
    for byte in &device_id.as_bytes()[..ALIAS_SUFFIX_ID_BYTES] {
        use std::fmt::Write;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn short_id_suffix(device_id: &DeviceId) -> String {
    format!("-{}", short_endpoint_id(device_id))
}

fn validate_alias(value: &str) -> Result<(), DeviceAliasError> {
    if value.is_empty() {
        return Err(DeviceAliasError::Empty);
    }
    if value.len() > MAX_DEVICE_ALIAS_BYTES {
        return Err(DeviceAliasError::TooLong {
            actual: value.len(),
            maximum: MAX_DEVICE_ALIAS_BYTES,
        });
    }
    if value.trim() != value {
        return Err(DeviceAliasError::SurroundingWhitespace);
    }
    if value.chars().any(char::is_control) {
        return Err(DeviceAliasError::ControlCharacter);
    }
    if value == RESERVED_DEVICE_ALIAS {
        return Err(DeviceAliasError::Reserved);
    }
    Ok(())
}

impl fmt::Display for DeviceAlias {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Failure while validating a device alias.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceAliasError {
    /// An alias must contain at least one byte.
    Empty,
    /// An alias exceeded the fixed UTF-8 byte bound.
    TooLong {
        /// Observed byte count.
        actual: usize,
        /// Maximum accepted byte count.
        maximum: usize,
    },
    /// Leading or trailing Unicode whitespace is forbidden.
    SurroundingWhitespace,
    /// Unicode control characters are forbidden.
    ControlCharacter,
    /// The reserved `local` value is not assignable.
    Reserved,
}

impl fmt::Display for DeviceAliasError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(formatter, "device alias must not be empty"),
            Self::TooLong { actual, maximum } => write!(
                formatter,
                "device alias must contain at most {maximum} UTF-8 bytes, got {actual}"
            ),
            Self::SurroundingWhitespace => {
                write!(
                    formatter,
                    "device alias must not have surrounding whitespace"
                )
            }
            Self::ControlCharacter => {
                write!(
                    formatter,
                    "device alias must not contain control characters"
                )
            }
            Self::Reserved => write!(
                formatter,
                "device alias must not use the reserved {RESERVED_DEVICE_ALIAS:?} value"
            ),
        }
    }
}

impl std::error::Error for DeviceAliasError {}

/// Validated HTTPS relay URL string used as a dial hint or ticket route.
///
/// Core deliberately does not depend on the Iroh URL type. It validates only
/// the product-admitted shape: a bounded `https://` string without control or
/// whitespace bytes. The owning Iroh adapter performs the real `RelayUrl`
/// parse, and canonicalization always retains the exact bytes stored here.
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RelayHint(String);

impl RelayHint {
    /// Validates a relay URL string.
    pub fn new(value: impl Into<String>) -> Result<Self, RelayHintError> {
        let value = value.into();
        if value.is_empty() {
            return Err(RelayHintError::Empty);
        }
        if value.len() > MAX_RELAY_URL_BYTES {
            return Err(RelayHintError::TooLong {
                actual: value.len(),
                maximum: MAX_RELAY_URL_BYTES,
            });
        }
        if !value.starts_with("https://") {
            return Err(RelayHintError::NotHttps);
        }
        if value
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
        {
            return Err(RelayHintError::ControlCharacter);
        }
        Ok(Self(value))
    }

    /// Borrows the validated URL string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for RelayHint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RelayHint")
            .field("url", &"[REDACTED]")
            .field("url_len", &self.0.len())
            .finish()
    }
}

impl fmt::Display for RelayHint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Failure while validating a relay hint URL.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RelayHintError {
    /// A relay URL must not be empty.
    Empty,
    /// A relay URL exceeded the fixed UTF-8 byte bound.
    TooLong {
        /// Observed byte count.
        actual: usize,
        /// Maximum accepted byte count.
        maximum: usize,
    },
    /// Only HTTPS relay URLs are admitted.
    NotHttps,
    /// Control or whitespace bytes are forbidden.
    ControlCharacter,
}

impl fmt::Display for RelayHintError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(formatter, "relay URL must not be empty"),
            Self::TooLong { actual, maximum } => write!(
                formatter,
                "relay URL must contain at most {maximum} UTF-8 bytes, got {actual}"
            ),
            Self::NotHttps => write!(formatter, "relay URL must use the https scheme"),
            Self::ControlCharacter => {
                write!(
                    formatter,
                    "relay URL must not contain control or whitespace bytes"
                )
            }
        }
    }
}

impl std::error::Error for RelayHintError {}

/// Failure while validating a directional device summary projection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceSummaryError {
    /// The outbound fields claim a known device without a known-device row.
    OutboundWithoutKnown,
    /// A known-device row carried an invalid remote display name.
    InvalidRemoteName(DeviceDisplayNameError),
    /// An inbound authorization or revocation carried generation zero.
    AuthWithZeroGeneration,
    /// The inbound status is absent but a non-zero generation is present.
    NoAuthWithGeneration,
    /// An inbound authorization or revocation omitted its pairing timestamp.
    AuthWithoutPairedTimestamp,
    /// An absent inbound authorization carried a pairing timestamp.
    NoAuthWithPairedTimestamp,
}

impl fmt::Display for DeviceSummaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutboundWithoutKnown => {
                write!(
                    formatter,
                    "outbound fields present without a known-device row"
                )
            }
            Self::InvalidRemoteName(error) => error.fmt(formatter),
            Self::AuthWithZeroGeneration => {
                write!(
                    formatter,
                    "authorized or revoked status requires a non-zero generation"
                )
            }
            Self::NoAuthWithGeneration => {
                write!(
                    formatter,
                    "no authorization status requires a zero generation"
                )
            }
            Self::AuthWithoutPairedTimestamp => write!(
                formatter,
                "authorized or revoked status requires a pairing timestamp"
            ),
            Self::NoAuthWithPairedTimestamp => {
                write!(
                    formatter,
                    "no authorization status requires a zero pairing timestamp"
                )
            }
        }
    }
}

impl std::error::Error for DeviceSummaryError {}

/// Directional projection of one device across the outbound address book and
/// the inbound authorization registry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceSummary {
    device_id: DeviceId,
    outbound_known: bool,
    alias: Option<DeviceAlias>,
    remote_name: String,
    route_verified: bool,
    auth_status: AuthorizationStatus,
    generation: AuthGeneration,
    paired_at_unix: u64,
    last_seen_at_unix: u64,
    online: bool,
    active_stream_count: u32,
    remote_attachment_count: u32,
}

impl DeviceSummary {
    /// Validates and constructs a consistent directional device projection.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        device_id: DeviceId,
        outbound_known: bool,
        alias: Option<DeviceAlias>,
        remote_name: impl Into<String>,
        route_verified: bool,
        auth_status: AuthorizationStatus,
        generation: AuthGeneration,
        paired_at_unix: u64,
        last_seen_at_unix: u64,
        online: bool,
        active_stream_count: u32,
        remote_attachment_count: u32,
    ) -> Result<Self, DeviceSummaryError> {
        let remote_name = remote_name.into();
        if !outbound_known && (alias.is_some() || !remote_name.is_empty() || route_verified) {
            return Err(DeviceSummaryError::OutboundWithoutKnown);
        }
        if outbound_known {
            DeviceDisplayName::new(remote_name.clone())
                .map_err(DeviceSummaryError::InvalidRemoteName)?;
        }
        match auth_status {
            AuthorizationStatus::None => {
                if generation != AuthGeneration::ZERO {
                    return Err(DeviceSummaryError::NoAuthWithGeneration);
                }
                if paired_at_unix != 0 {
                    return Err(DeviceSummaryError::NoAuthWithPairedTimestamp);
                }
            }
            AuthorizationStatus::Authorized | AuthorizationStatus::Revoked => {
                if generation == AuthGeneration::ZERO {
                    return Err(DeviceSummaryError::AuthWithZeroGeneration);
                }
                if paired_at_unix == 0 {
                    return Err(DeviceSummaryError::AuthWithoutPairedTimestamp);
                }
            }
        }
        Ok(Self {
            device_id,
            outbound_known,
            alias,
            remote_name,
            route_verified,
            auth_status,
            generation,
            paired_at_unix,
            last_seen_at_unix,
            online,
            active_stream_count,
            remote_attachment_count,
        })
    }

    /// Device identity.
    #[must_use]
    pub const fn device_id(&self) -> DeviceId {
        self.device_id
    }

    /// Whether an outbound known-device row exists.
    #[must_use]
    pub const fn outbound_known(&self) -> bool {
        self.outbound_known
    }

    /// Outbound local alias, if one is assigned.
    #[must_use]
    pub const fn alias(&self) -> Option<&DeviceAlias> {
        self.alias.as_ref()
    }

    /// Outbound remote display name.
    #[must_use]
    pub fn remote_name(&self) -> &str {
        &self.remote_name
    }

    /// Whether the outbound relay route was verified.
    #[must_use]
    pub const fn route_verified(&self) -> bool {
        self.route_verified
    }

    /// Inbound authorization status.
    #[must_use]
    pub const fn auth_status(&self) -> AuthorizationStatus {
        self.auth_status
    }

    /// Inbound authorization generation.
    #[must_use]
    pub const fn generation(&self) -> AuthGeneration {
        self.generation
    }

    /// Inbound pairing timestamp.
    #[must_use]
    pub const fn paired_at_unix(&self) -> u64 {
        self.paired_at_unix
    }

    /// Last-seen timestamp.
    #[must_use]
    pub const fn last_seen_at_unix(&self) -> u64 {
        self.last_seen_at_unix
    }

    /// Whether a live primary connection is present.
    #[must_use]
    pub const fn online(&self) -> bool {
        self.online
    }

    /// Live stream count.
    #[must_use]
    pub const fn active_stream_count(&self) -> u32 {
        self.active_stream_count
    }

    /// Live remote attachment count.
    #[must_use]
    pub const fn remote_attachment_count(&self) -> u32 {
        self.remote_attachment_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_name_is_a_shared_bounded_validation_boundary() {
        let name = DeviceDisplayName::new("Work laptop").expect("valid display name");
        assert_eq!(name.as_str(), "Work laptop");
        assert_eq!(name.into_string(), "Work laptop");
        assert_eq!(
            DeviceDisplayName::new(""),
            Err(DeviceDisplayNameError::Empty)
        );
        assert_eq!(
            DeviceDisplayName::new("line\nbreak"),
            Err(DeviceDisplayNameError::ControlCharacter)
        );
        assert!(matches!(
            DeviceDisplayName::new("界".repeat(43)),
            Err(DeviceDisplayNameError::TooLong {
                actual: 129,
                maximum: MAX_DEVICE_DISPLAY_NAME_BYTES
            })
        ));
    }

    #[test]
    fn alias_rejects_invalid_and_reserved_values() {
        assert_eq!(DeviceAlias::new(""), Err(DeviceAliasError::Empty));
        assert_eq!(
            DeviceAlias::new(" padded"),
            Err(DeviceAliasError::SurroundingWhitespace)
        );
        assert_eq!(
            DeviceAlias::new("line\nbreak"),
            Err(DeviceAliasError::ControlCharacter)
        );
        assert_eq!(DeviceAlias::new("local"), Err(DeviceAliasError::Reserved));
        assert!(matches!(
            DeviceAlias::new("界".repeat(43)),
            Err(DeviceAliasError::TooLong {
                actual: 129,
                maximum: 128
            })
        ));
        let alias = DeviceAlias::new("Work-界").expect("bounded Unicode alias");
        assert_eq!(alias.as_str(), "Work-界");
        assert_ne!(alias, DeviceAlias::new("work-界").expect("case-sensitive"));
        assert_eq!(
            format!("{alias}"),
            "Work-界",
            "Display does not escape the alias"
        );
    }

    #[test]
    fn default_alias_suffixes_short_endpoint_id_without_overflowing() {
        let device = DeviceId::from_array([0xab; 32]);
        assert_eq!(short_endpoint_id(&device), "abababab");

        // A clean remote name becomes the alias unchanged.
        assert_eq!(
            DeviceAlias::from_remote_name("laptop").expect("clean name"),
            DeviceAlias::new("laptop").expect("clean name")
        );
        assert_eq!(DeviceAlias::from_remote_name("local"), None);
        assert_eq!(DeviceAlias::from_remote_name(""), None);

        // Reserved/empty names fall back to the disambiguated form.
        let fallback = DeviceAlias::disambiguated("local", &device);
        assert_eq!(fallback.as_str(), "local-abababab");

        // A name at the byte ceiling is truncated to keep the suffix within bounds.
        let long = "界".repeat(64);
        let disambiguated = DeviceAlias::disambiguated(&long, &device);
        assert!(disambiguated.as_str().ends_with("-abababab"));
        assert!(disambiguated.as_str().len() <= MAX_DEVICE_ALIAS_BYTES);
        assert_ne!(disambiguated.as_str(), long.as_str());
    }

    #[test]
    fn disambiguated_always_yields_a_valid_alias() {
        let device = DeviceId::from_array([0xab; 32]);
        for remote_name in [
            "  laptop",    // leading whitespace
            "laptop  ",    // trailing whitespace
            "  laptop  ",  // surrounding whitespace
            "\tlaptop",    // leading tab
            "line\nbreak", // interior control character
            "\u{1}laptop", // leading non-whitespace control
            "  ",          // only whitespace
            "",            // empty
            "local",       // reserved bare value
        ] {
            let alias = DeviceAlias::disambiguated(remote_name, &device);
            DeviceAlias::new(alias.as_str()).expect("disambiguated alias must be valid");
            assert!(
                alias.as_str().ends_with("-abababab"),
                "alias {alias:?} must keep its disambiguating suffix"
            );
        }
    }

    #[test]
    fn relay_hint_requires_bounded_https_url() {
        let url = RelayHint::new("https://relay.example.com").expect("bounded https URL");
        assert_eq!(url.as_str(), "https://relay.example.com");
        assert_eq!(RelayHint::new(""), Err(RelayHintError::Empty));
        assert_eq!(
            RelayHint::new("http://relay.example.com"),
            Err(RelayHintError::NotHttps)
        );
        assert_eq!(
            RelayHint::new("https://relay.example.com/a b"),
            Err(RelayHintError::ControlCharacter)
        );
        assert_eq!(
            RelayHint::new(format!("https://{}", "r".repeat(MAX_RELAY_URL_BYTES))),
            Err(RelayHintError::TooLong {
                actual: 8 + MAX_RELAY_URL_BYTES,
                maximum: MAX_RELAY_URL_BYTES
            })
        );

        let sentinel = "https://relay-route-sentinel-7b31.example.test/path";
        let hint = RelayHint::new(sentinel).expect("sentinel is a valid Relay hint");
        let rendered = format!("{hint:?}");
        assert!(rendered.contains("RelayHint"));
        assert!(rendered.contains("[REDACTED]"));
        assert!(rendered.contains(&format!("url_len: {}", sentinel.len())));
        assert!(!rendered.contains(sentinel));
        assert_eq!(hint.as_str(), sentinel);
        assert_eq!(hint.to_string(), sentinel);
    }
}
