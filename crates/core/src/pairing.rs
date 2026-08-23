//! One-time pairing domain: tickets, canonical transcripts, and HMAC proofs.
//!
//! The canonical bytes below are the authentication input. They never depend on
//! protobuf serialization order; they use fixed big-endian length prefixes and
//! domain-separated HMAC-SHA256 so any language can reproduce the same vectors.

use std::fmt;

use ring::digest::{Context, SHA256, digest};
use ring::hmac::{HMAC_SHA256, Key, sign, verify};
use zeroize::Zeroizing;

use crate::{
    AuthGeneration, DeviceAlias, DeviceDisplayName, DeviceDisplayNameError, DeviceId, PairNonce,
    PairOfferId, RelayHint,
};

/// Only supported ticket format version.
pub const PAIR_TICKET_FORMAT_VERSION: u32 = 1;
/// Only supported pair handshake protocol version.
pub const PAIR_PROTOCOL_VERSION: u32 = 1;
/// Byte width of a pairing ticket secret.
pub const PAIR_SECRET_BYTES: usize = 32;
/// Byte width of a pairing offer identifier.
pub const PAIR_OFFER_ID_BYTES: usize = 16;
/// Byte width of a pairing challenge nonce.
pub const PAIR_NONCE_BYTES: usize = 32;
/// Maximum encoded UTF-8 bytes in a device display name.
pub const MAX_DEVICE_NAME_BYTES: usize = crate::MAX_DEVICE_DISPLAY_NAME_BYTES;
/// Default ticket lifetime in seconds.
pub const DEFAULT_PAIR_TTL_SECONDS: u64 = 600;
/// Minimum allowed ticket lifetime in seconds.
pub const MIN_PAIR_TTL_SECONDS: u64 = 60;
/// Maximum allowed ticket lifetime in seconds.
pub const MAX_PAIR_TTL_SECONDS: u64 = 3600;
/// Exact byte width of a local pairing operation semantic fingerprint.
///
/// The fingerprint is always a 256-bit digest, so a client cannot smuggle
/// arbitrary or ticket-bearing bytes into a value that gets logged.
pub const PAIR_FINGERPRINT_BYTES: usize = 32;

const TICKET_DOMAIN: &[u8] = b"zterm-pair-ticket-v1\0";
const OFFER_KEY_DOMAIN: &[u8] = b"zterm-pair-offer-key-v1\0";
const TRANSCRIPT_DOMAIN: &[u8] = b"zterm-pair-transcript-v1\0";
const CONTROLLER_PROOF_DOMAIN: &[u8] = b"zterm-pair-controller-proof-v1\0";
const HOST_ACCEPTED_DOMAIN: &[u8] = b"zterm-pair-host-accepted-v1\0";
const CREATE_FINGERPRINT_DOMAIN: &[u8] = b"zterm-local-pair-create-fingerprint-v1\0";
const ACCEPT_FINGERPRINT_DOMAIN: &[u8] = b"zterm-local-pair-accept-fingerprint-v1\0";

/// Redacted, zeroizing bearer secret from a pairing ticket.
#[derive(Clone, Eq, PartialEq)]
pub struct PairSecret(Zeroizing<[u8; PAIR_SECRET_BYTES]>);

impl PairSecret {
    /// Constructs a secret from its exact 32 bytes.
    #[must_use]
    pub fn from_bytes(bytes: [u8; PAIR_SECRET_BYTES]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    /// Validates and copies a secret from a byte slice.
    pub fn from_slice(bytes: &[u8]) -> Result<Self, PairSecretError> {
        let bytes: [u8; PAIR_SECRET_BYTES] =
            bytes
                .try_into()
                .map_err(|_| PairSecretError::InvalidLength {
                    actual: bytes.len(),
                })?;
        Ok(Self::from_bytes(bytes))
    }

    /// Borrows the secret bytes for HMAC key derivation.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; PAIR_SECRET_BYTES] {
        &self.0
    }
}

impl fmt::Debug for PairSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PairSecret([REDACTED])")
    }
}

impl fmt::Display for PairSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

/// Failure while validating a pairing secret.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PairSecretError {
    /// The secret had the wrong byte width.
    InvalidLength {
        /// Observed byte count.
        actual: usize,
    },
}

impl fmt::Display for PairSecretError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { actual } => write!(
                formatter,
                "pair secret must contain exactly {PAIR_SECRET_BYTES} bytes, got {actual}"
            ),
        }
    }
}

impl std::error::Error for PairSecretError {}

/// Failure while validating pairing ticket or transcript fields.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PairTicketError {
    /// The ticket format version is not supported.
    UnsupportedFormatVersion {
        /// Observed format version.
        actual: u32,
    },
    /// The pair handshake protocol version is not supported.
    UnsupportedProtocolVersion {
        /// Observed protocol version.
        actual: u32,
    },
    /// A device display name is empty.
    EmptyDeviceName,
    /// A device display name exceeded its UTF-8 byte bound.
    DeviceNameTooLong {
        /// Observed byte count.
        actual: usize,
    },
    /// A device display name contained a control character.
    DeviceNameControl,
    /// A ticket must advertise at least one relay hint.
    MissingRelayHint,
    /// A ticket advertised more relay hints than the product bound.
    TooManyRelayHints {
        /// Observed count.
        actual: usize,
    },
    /// A ticket repeated the same relay hint URL.
    DuplicateRelayHint,
    /// A pairing TTL fell outside the bounded product range.
    TtlOutOfRange {
        /// Observed TTL in seconds.
        actual: u64,
    },
    /// A committed pair acceptance used the reserved zero generation.
    ZeroAuthorizationGeneration,
    /// A canonical field exceeded its fixed big-endian length-prefix width.
    CanonicalLengthOverflow,
}

impl fmt::Display for PairTicketError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedFormatVersion { actual } => write!(
                formatter,
                "unsupported pairing ticket format version {actual}"
            ),
            Self::UnsupportedProtocolVersion { actual } => {
                write!(formatter, "unsupported pairing protocol version {actual}")
            }
            Self::EmptyDeviceName => write!(formatter, "device name must not be empty"),
            Self::DeviceNameTooLong { actual } => write!(
                formatter,
                "device name must contain at most {MAX_DEVICE_NAME_BYTES} UTF-8 bytes, got {actual}"
            ),
            Self::DeviceNameControl => {
                write!(formatter, "device name must not contain control characters")
            }
            Self::MissingRelayHint => write!(formatter, "ticket must advertise a relay hint"),
            Self::TooManyRelayHints { actual } => {
                write!(
                    formatter,
                    "ticket advertised too many relay hints ({actual})"
                )
            }
            Self::DuplicateRelayHint => {
                write!(formatter, "ticket repeated the same relay hint URL")
            }
            Self::TtlOutOfRange { actual } => write!(
                formatter,
                "pairing TTL must be within {MIN_PAIR_TTL_SECONDS}..={MAX_PAIR_TTL_SECONDS} seconds, got {actual}"
            ),
            Self::ZeroAuthorizationGeneration => {
                write!(
                    formatter,
                    "pair acceptance requires a non-zero authorization generation"
                )
            }
            Self::CanonicalLengthOverflow => write!(
                formatter,
                "a canonical field length exceeded its fixed big-endian prefix width"
            ),
        }
    }
}

impl std::error::Error for PairTicketError {}

/// Validates a requested pairing TTL against the bounded product range.
pub fn validate_pair_ttl(seconds: u64) -> Result<(), PairTicketError> {
    if (MIN_PAIR_TTL_SECONDS..=MAX_PAIR_TTL_SECONDS).contains(&seconds) {
        Ok(())
    } else {
        Err(PairTicketError::TtlOutOfRange { actual: seconds })
    }
}

/// Fixed-width semantic fingerprint of one local pairing operation.
///
/// The fingerprint is always a 256-bit digest of the mutation arguments (for
/// accept, a hash over the ticket and alias), never the raw payload, so it can
/// never carry the ticket secret. Its [`Debug`] projection is fully redacted,
/// so a hostile client cannot stuff arbitrary bytes into something that is
/// later logged. It is validated before any service allocation and
/// distinguishes "same operation ID, different payload".
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct PairFingerprint([u8; PAIR_FINGERPRINT_BYTES]);

impl PairFingerprint {
    /// Computes the semantic fingerprint for one local pair-create operation.
    ///
    /// The exact language-neutral input is:
    ///
    /// ```text
    /// "zterm-local-pair-create-fingerprint-v1\0"
    /// u64be effective_ttl_seconds
    /// ```
    ///
    /// Callers must resolve the wire value `0` to the effective default TTL
    /// before calling this helper, so client and server compare semantics rather
    /// than two spellings of the same request.
    #[must_use]
    pub fn for_create(effective_ttl_seconds: u64) -> Self {
        let mut context = Context::new(&SHA256);
        context.update(CREATE_FINGERPRINT_DOMAIN);
        context.update(&effective_ttl_seconds.to_be_bytes());
        Self::from_digest(context.finish())
    }

    /// Computes the semantic fingerprint for one local pair-accept operation.
    ///
    /// The exact language-neutral input is:
    ///
    /// ```text
    /// "zterm-local-pair-accept-fingerprint-v1\0"
    /// u64be ticket_text_byte_length
    /// ticket_text_bytes
    /// u8 explicit_alias_present
    /// [u16be explicit_alias_byte_length + explicit_alias_utf8]
    /// ```
    ///
    /// Ticket bytes are fed directly into SHA-256; this helper retains no
    /// ticket-sized canonical buffer and returns only the 32-byte digest. The
    /// wire boundary remains responsible for rejecting ticket text over its
    /// product limit before allocating a service operation cell.
    #[must_use]
    pub fn for_accept(ticket_text_bytes: &[u8], explicit_alias: Option<&DeviceAlias>) -> Self {
        let ticket_length = u64::try_from(ticket_text_bytes.len())
            .expect("Rust slice lengths fit in the fingerprint's u64 length field");
        let mut context = Context::new(&SHA256);
        context.update(ACCEPT_FINGERPRINT_DOMAIN);
        context.update(&ticket_length.to_be_bytes());
        context.update(ticket_text_bytes);
        match explicit_alias {
            None => context.update(&[0]),
            Some(alias) => {
                let bytes = alias.as_str().as_bytes();
                let length = u16::try_from(bytes.len())
                    .expect("validated device aliases fit in a u16 length field");
                context.update(&[1]);
                context.update(&length.to_be_bytes());
                context.update(bytes);
            }
        }
        Self::from_digest(context.finish())
    }

    /// Constructs a fingerprint from its exact 32 digest bytes.
    #[must_use]
    pub fn from_bytes(bytes: [u8; PAIR_FINGERPRINT_BYTES]) -> Self {
        Self(bytes)
    }

    /// Validates and copies a fingerprint from a byte slice.
    pub fn from_slice(bytes: &[u8]) -> Result<Self, PairFingerprintError> {
        let bytes: [u8; PAIR_FINGERPRINT_BYTES] =
            bytes
                .try_into()
                .map_err(|_| PairFingerprintError::InvalidLength {
                    actual: bytes.len(),
                })?;
        Ok(Self::from_bytes(bytes))
    }

    /// Borrows the fingerprint bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; PAIR_FINGERPRINT_BYTES] {
        &self.0
    }

    fn from_digest(digest: ring::digest::Digest) -> Self {
        let bytes = digest
            .as_ref()
            .try_into()
            .expect("SHA-256 always returns a 32-byte digest");
        Self::from_bytes(bytes)
    }
}

impl fmt::Debug for PairFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PairFingerprint([REDACTED])")
    }
}

/// Failure while validating a pairing operation semantic fingerprint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PairFingerprintError {
    /// The fingerprint had the wrong byte width.
    InvalidLength {
        /// Observed byte count.
        actual: usize,
    },
}

impl fmt::Display for PairFingerprintError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { actual } => write!(
                formatter,
                "pairing operation fingerprint must contain exactly {PAIR_FINGERPRINT_BYTES} bytes, got {actual}"
            ),
        }
    }
}

impl std::error::Error for PairFingerprintError {}

/// Public, non-secret fields of a versioned one-time pairing ticket.
///
/// The canonical secret-free bytes are computed once during construction so
/// the digest and offer-key derivations stay infallible without any truncating
/// cast in the authentication path.
#[derive(Clone, Eq, PartialEq)]
pub struct PairTicketFields {
    format_version: u32,
    host_device_id: DeviceId,
    host_name: DeviceDisplayName,
    relay_hints: Vec<RelayHint>,
    offer_id: PairOfferId,
    expires_at_unix: u64,
    canonical: Vec<u8>,
}

impl fmt::Debug for PairTicketFields {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PairTicketFields")
            .field("format_version", &self.format_version)
            .field("host_device_id", &self.host_device_id)
            .field("host_name", &self.host_name)
            .field("relay_hints", &self.relay_hints)
            .field("offer_id", &self.offer_id)
            .field("expires_at_unix", &self.expires_at_unix)
            .finish_non_exhaustive()
    }
}

impl PairTicketFields {
    /// Validates and constructs public ticket fields.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        format_version: u32,
        host_device_id: DeviceId,
        host_name: impl Into<String>,
        relay_hints: Vec<RelayHint>,
        offer_id: PairOfferId,
        expires_at_unix: u64,
    ) -> Result<Self, PairTicketError> {
        if format_version != PAIR_TICKET_FORMAT_VERSION {
            return Err(PairTicketError::UnsupportedFormatVersion {
                actual: format_version,
            });
        }
        let host_name = parse_device_name(host_name)?;
        validate_relay_hints(&relay_hints)?;
        let canonical = canonical_ticket_bytes(
            format_version,
            &host_device_id,
            host_name.as_str(),
            &relay_hints,
            &offer_id,
            expires_at_unix,
        )?;
        Ok(Self {
            format_version,
            host_device_id,
            host_name,
            relay_hints,
            offer_id,
            expires_at_unix,
            canonical,
        })
    }

    /// Ticket format version.
    #[must_use]
    pub const fn format_version(&self) -> u32 {
        self.format_version
    }

    /// Host device identity.
    #[must_use]
    pub const fn host_device_id(&self) -> DeviceId {
        self.host_device_id
    }

    /// Host display name.
    #[must_use]
    pub fn host_name(&self) -> &str {
        self.host_name.as_str()
    }

    /// Ordered relay hint URLs.
    #[must_use]
    pub fn relay_hints(&self) -> &[RelayHint] {
        &self.relay_hints
    }

    /// One-time offer identifier.
    #[must_use]
    pub const fn offer_id(&self) -> PairOfferId {
        self.offer_id
    }

    /// Absolute Unix expiry timestamp.
    #[must_use]
    pub const fn expires_at_unix(&self) -> u64 {
        self.expires_at_unix
    }

    /// Returns whether the ticket has expired at `now_unix`.
    #[must_use]
    pub const fn is_expired(&self, now_unix: u64) -> bool {
        now_unix >= self.expires_at_unix
    }

    /// Canonical secret-free ticket bytes used for the digest and offer key.
    #[must_use]
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical
    }

    /// SHA-256 digest of the canonical secret-free ticket bytes.
    #[must_use]
    pub fn ticket_digest(&self) -> [u8; 32] {
        sha256(&self.canonical)
    }

    /// HMAC-SHA256 offer key derived from the ticket secret.
    ///
    /// The pairing manager stores only this derived key, never the raw secret.
    #[must_use]
    pub fn offer_key(&self, secret: &PairSecret) -> [u8; 32] {
        let mut data = Vec::with_capacity(OFFER_KEY_DOMAIN.len() + self.canonical.len());
        data.extend_from_slice(OFFER_KEY_DOMAIN);
        data.extend_from_slice(&self.canonical);
        hmac_sha256(secret.as_bytes(), &data)
    }
}

/// Canonical transcript binding one pairing handshake.
///
/// Like [`PairTicketFields`], the canonical bytes are computed once during
/// construction so proof/confirmation derivation stays infallible.
#[derive(Clone, Eq, PartialEq)]
pub struct PairTranscript {
    ticket_digest: [u8; 32],
    host_device_id: DeviceId,
    controller_device_id: DeviceId,
    offer_id: PairOfferId,
    controller_nonce: PairNonce,
    host_nonce: PairNonce,
    controller_name: DeviceDisplayName,
    ticket_format_version: u32,
    pair_protocol_version: u32,
    expires_at_unix: u64,
    canonical: Vec<u8>,
}

impl fmt::Debug for PairTranscript {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PairTranscript")
            .field("ticket_digest", &self.ticket_digest)
            .field("host_device_id", &self.host_device_id)
            .field("controller_device_id", &self.controller_device_id)
            .field("offer_id", &self.offer_id)
            .field("controller_nonce", &self.controller_nonce)
            .field("host_nonce", &self.host_nonce)
            .field("controller_name", &self.controller_name)
            .field("ticket_format_version", &self.ticket_format_version)
            .field("pair_protocol_version", &self.pair_protocol_version)
            .field("expires_at_unix", &self.expires_at_unix)
            .finish_non_exhaustive()
    }
}

impl PairTranscript {
    /// Binds ticket fields with the authenticated handshake inputs.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ticket: &PairTicketFields,
        controller_device_id: DeviceId,
        controller_name: impl Into<String>,
        controller_nonce: PairNonce,
        host_nonce: PairNonce,
        pair_protocol_version: u32,
    ) -> Result<Self, PairTicketError> {
        validate_protocol_version(pair_protocol_version)?;
        let controller_name = parse_device_name(controller_name)?;
        let ticket_digest = ticket.ticket_digest();
        let canonical = canonical_transcript_bytes(
            &ticket_digest,
            &ticket.host_device_id,
            &controller_device_id,
            &ticket.offer_id,
            &controller_nonce,
            &host_nonce,
            controller_name.as_str(),
            ticket.format_version,
            pair_protocol_version,
            ticket.expires_at_unix,
        )?;
        Ok(Self {
            ticket_digest,
            host_device_id: ticket.host_device_id,
            controller_device_id,
            offer_id: ticket.offer_id,
            controller_nonce,
            host_nonce,
            controller_name,
            ticket_format_version: ticket.format_version,
            pair_protocol_version,
            expires_at_unix: ticket.expires_at_unix,
            canonical,
        })
    }

    /// Canonical transcript bytes bound by the controller proof.
    #[must_use]
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical
    }

    /// Controller proof of ticket-secret possession.
    #[must_use]
    pub fn controller_proof(&self, offer_key: &[u8; 32]) -> [u8; 32] {
        hmac_sha256(offer_key, &self.controller_proof_data())
    }

    /// Constant-time controller proof verification.
    #[must_use]
    pub fn verify_controller_proof(&self, offer_key: &[u8; 32], proof: &[u8; 32]) -> bool {
        verify_hmac_sha256(offer_key, &self.controller_proof_data(), proof)
    }

    /// Host acceptance confirmation bound to the resulting authorization generation.
    #[must_use]
    pub fn host_confirmation(&self, offer_key: &[u8; 32], generation: u64) -> [u8; 32] {
        hmac_sha256(offer_key, &self.host_confirmation_data(generation))
    }

    /// Constant-time host confirmation verification.
    #[must_use]
    pub fn verify_host_confirmation(
        &self,
        offer_key: &[u8; 32],
        generation: u64,
        proof: &[u8; 32],
    ) -> bool {
        verify_hmac_sha256(offer_key, &self.host_confirmation_data(generation), proof)
    }

    fn controller_proof_data(&self) -> Vec<u8> {
        let mut data = Vec::with_capacity(CONTROLLER_PROOF_DOMAIN.len() + self.canonical.len());
        data.extend_from_slice(CONTROLLER_PROOF_DOMAIN);
        data.extend_from_slice(&self.canonical);
        data
    }

    fn host_confirmation_data(&self, generation: u64) -> Vec<u8> {
        let mut data = Vec::with_capacity(HOST_ACCEPTED_DOMAIN.len() + self.canonical.len());
        data.extend_from_slice(HOST_ACCEPTED_DOMAIN);
        data.extend_from_slice(&self.canonical);
        push_u64be(&mut data, generation);
        data
    }
}

/// Controller opening of a pairing handshake.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PairBegin {
    offer_id: PairOfferId,
    controller_name: DeviceDisplayName,
    controller_nonce: PairNonce,
    pair_protocol_version: u32,
}

impl PairBegin {
    /// Validates the controller opening of a pairing handshake.
    pub fn new(
        offer_id: PairOfferId,
        controller_name: impl Into<String>,
        controller_nonce: PairNonce,
        pair_protocol_version: u32,
    ) -> Result<Self, PairTicketError> {
        let controller_name = parse_device_name(controller_name)?;
        validate_protocol_version(pair_protocol_version)?;
        Ok(Self {
            offer_id,
            controller_name,
            controller_nonce,
            pair_protocol_version,
        })
    }

    /// One-time offer identifier.
    #[must_use]
    pub const fn offer_id(&self) -> PairOfferId {
        self.offer_id
    }

    /// Controller display name.
    #[must_use]
    pub fn controller_name(&self) -> &str {
        self.controller_name.as_str()
    }

    /// Controller challenge nonce.
    #[must_use]
    pub const fn controller_nonce(&self) -> PairNonce {
        self.controller_nonce
    }

    /// Negotiated pair protocol version.
    #[must_use]
    pub const fn pair_protocol_version(&self) -> u32 {
        self.pair_protocol_version
    }
}

/// Host challenge half of a pairing handshake.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PairChallenge {
    host_nonce: PairNonce,
    selected_version: u32,
    ticket_expiry_unix: u64,
}

impl PairChallenge {
    /// Validates the host challenge half of a pairing handshake.
    pub fn new(
        host_nonce: PairNonce,
        selected_version: u32,
        ticket_expiry_unix: u64,
    ) -> Result<Self, PairTicketError> {
        validate_protocol_version(selected_version)?;
        Ok(Self {
            host_nonce,
            selected_version,
            ticket_expiry_unix,
        })
    }

    /// Host challenge nonce.
    #[must_use]
    pub const fn host_nonce(&self) -> PairNonce {
        self.host_nonce
    }

    /// Selected pair protocol version.
    #[must_use]
    pub const fn selected_version(&self) -> u32 {
        self.selected_version
    }

    /// Absolute ticket expiry echoed back to the controller.
    #[must_use]
    pub const fn ticket_expiry_unix(&self) -> u64 {
        self.ticket_expiry_unix
    }
}

/// Controller proof of ticket-secret possession (a 256-bit HMAC).
pub struct PairProof(Zeroizing<[u8; PAIR_SECRET_BYTES]>);

impl PairProof {
    /// Constructs a proof from its exact 32 bytes.
    #[must_use]
    pub fn from_bytes(bytes: [u8; PAIR_SECRET_BYTES]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    /// Validates and copies a proof from a byte slice.
    pub fn from_slice(bytes: &[u8]) -> Result<Self, crate::IdLengthError> {
        let bytes: [u8; PAIR_SECRET_BYTES] = bytes
            .try_into()
            .map_err(|_| crate::IdLengthError::new("PairProof", PAIR_SECRET_BYTES, bytes.len()))?;
        Ok(Self::from_bytes(bytes))
    }

    /// Borrows the proof bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; PAIR_SECRET_BYTES] {
        &self.0
    }
}

impl Clone for PairProof {
    fn clone(&self) -> Self {
        Self::from_bytes(*self.as_bytes())
    }
}

impl PartialEq for PairProof {
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for PairProof {}

impl fmt::Debug for PairProof {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PairProof([REDACTED])")
    }
}

/// Host acceptance confirmation of a pairing handshake.
pub struct PairAccepted {
    authorization_generation: AuthGeneration,
    host_confirmation_proof: Zeroizing<[u8; PAIR_SECRET_BYTES]>,
    host_diagnostic_version: String,
}

impl PairAccepted {
    /// Validates the host acceptance confirmation.
    pub fn new(
        authorization_generation: AuthGeneration,
        host_confirmation_proof: [u8; PAIR_SECRET_BYTES],
        host_diagnostic_version: impl Into<String>,
    ) -> Result<Self, PairTicketError> {
        Self::from_proof(
            authorization_generation,
            PairProof::from_bytes(host_confirmation_proof),
            host_diagnostic_version,
        )
    }

    /// Validates an acceptance while taking ownership of an already-zeroizing
    /// proof value. Protocol adapters use this form so a generated proof buffer
    /// never falls back to an ordinary `Vec` or array owner on error paths.
    pub fn from_proof(
        authorization_generation: AuthGeneration,
        host_confirmation_proof: PairProof,
        host_diagnostic_version: impl Into<String>,
    ) -> Result<Self, PairTicketError> {
        if authorization_generation == AuthGeneration::ZERO {
            return Err(PairTicketError::ZeroAuthorizationGeneration);
        }
        let host_diagnostic_version = host_diagnostic_version.into();
        validate_device_name(&host_diagnostic_version)?;
        Ok(Self {
            authorization_generation,
            host_confirmation_proof: host_confirmation_proof.0,
            host_diagnostic_version,
        })
    }

    /// Resulting inbound authorization generation.
    #[must_use]
    pub const fn authorization_generation(&self) -> AuthGeneration {
        self.authorization_generation
    }

    /// Host confirmation proof bytes.
    #[must_use]
    pub fn host_confirmation_proof(&self) -> &[u8; PAIR_SECRET_BYTES] {
        &self.host_confirmation_proof
    }

    /// Host diagnostic build version.
    #[must_use]
    pub fn host_diagnostic_version(&self) -> &str {
        &self.host_diagnostic_version
    }
}

impl Clone for PairAccepted {
    fn clone(&self) -> Self {
        Self {
            authorization_generation: self.authorization_generation,
            host_confirmation_proof: Zeroizing::new(*self.host_confirmation_proof),
            host_diagnostic_version: self.host_diagnostic_version.clone(),
        }
    }
}

impl PartialEq for PairAccepted {
    fn eq(&self, other: &Self) -> bool {
        self.authorization_generation == other.authorization_generation
            && self.host_confirmation_proof.as_ref() == other.host_confirmation_proof.as_ref()
            && self.host_diagnostic_version == other.host_diagnostic_version
    }
}

impl Eq for PairAccepted {}

impl fmt::Debug for PairAccepted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PairAccepted")
            .field("authorization_generation", &self.authorization_generation)
            .field("host_confirmation_proof", &"[REDACTED]")
            .field("host_diagnostic_version", &self.host_diagnostic_version)
            .finish()
    }
}

/// Bounded accumulator for one pairing handshake byte budget.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PairHandshakeBudget {
    used: usize,
    maximum: usize,
}

impl Default for PairHandshakeBudget {
    fn default() -> Self {
        Self::new()
    }
}

impl PairHandshakeBudget {
    /// Starts an empty handshake using the production 64 KiB ceiling.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            used: 0,
            maximum: crate::transport::MAX_PAIR_HANDSHAKE_BYTES,
        }
    }

    /// Starts an empty handshake with a validated injected ceiling.
    pub const fn with_maximum(maximum: usize) -> Result<Self, PairHandshakeBudgetError> {
        if maximum == 0 {
            return Err(PairHandshakeBudgetError::InvalidMaximum);
        }
        Ok(Self { used: 0, maximum })
    }

    /// Bytes already accounted for.
    #[must_use]
    pub const fn used(&self) -> usize {
        self.used
    }

    /// Maximum bytes accepted by this accumulator.
    #[must_use]
    pub const fn maximum(&self) -> usize {
        self.maximum
    }

    /// Remaining budget before the handshake exceeds its ceiling.
    #[must_use]
    pub const fn remaining(&self) -> usize {
        self.maximum.saturating_sub(self.used)
    }

    /// Accounts one handshake frame's byte count with checked arithmetic.
    ///
    /// Rejects the frame once the cumulative total would exceed the configured
    /// ceiling or overflow `usize`.
    pub fn record(&mut self, bytes: usize) -> Result<(), PairHandshakeBudgetError> {
        let used = self
            .used
            .checked_add(bytes)
            .ok_or(PairHandshakeBudgetError::Overflow)?;
        if used > self.maximum {
            return Err(PairHandshakeBudgetError::Exceeded {
                used,
                maximum: self.maximum,
            });
        }
        self.used = used;
        Ok(())
    }
}

/// Failure while accounting a pairing handshake byte budget.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PairHandshakeBudgetError {
    /// A byte ceiling of zero would reject every handshake.
    InvalidMaximum,
    /// The cumulative byte count overflowed `usize`.
    Overflow,
    /// The cumulative byte count exceeded the configured handshake ceiling.
    Exceeded {
        /// Observed cumulative byte count.
        used: usize,
        /// Configured byte ceiling.
        maximum: usize,
    },
}

impl fmt::Display for PairHandshakeBudgetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMaximum => {
                write!(formatter, "pairing handshake byte ceiling must be non-zero")
            }
            Self::Overflow => write!(formatter, "pairing handshake byte budget overflow"),
            Self::Exceeded { used, maximum } => write!(
                formatter,
                "pairing handshake bytes {used} exceed the {maximum} byte ceiling"
            ),
        }
    }
}

impl std::error::Error for PairHandshakeBudgetError {}

fn validate_protocol_version(version: u32) -> Result<(), PairTicketError> {
    if version != PAIR_PROTOCOL_VERSION {
        Err(PairTicketError::UnsupportedProtocolVersion { actual: version })
    } else {
        Ok(())
    }
}

fn parse_device_name(name: impl Into<String>) -> Result<DeviceDisplayName, PairTicketError> {
    DeviceDisplayName::new(name).map_err(|error| match error {
        DeviceDisplayNameError::Empty => PairTicketError::EmptyDeviceName,
        DeviceDisplayNameError::TooLong { actual, .. } => {
            PairTicketError::DeviceNameTooLong { actual }
        }
        DeviceDisplayNameError::ControlCharacter => PairTicketError::DeviceNameControl,
    })
}

pub(crate) fn validate_device_name(name: &str) -> Result<(), PairTicketError> {
    parse_device_name(name.to_owned()).map(drop)
}

fn validate_relay_hints(hints: &[RelayHint]) -> Result<(), PairTicketError> {
    if hints.is_empty() {
        return Err(PairTicketError::MissingRelayHint);
    }
    if hints.len() > crate::transport::MAX_RELAY_HINTS {
        return Err(PairTicketError::TooManyRelayHints {
            actual: hints.len(),
        });
    }
    for (index, hint) in hints.iter().enumerate() {
        if hints[..index].contains(hint) {
            return Err(PairTicketError::DuplicateRelayHint);
        }
    }
    Ok(())
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    let digest = digest(&SHA256, bytes);
    let mut out = [0_u8; 32];
    out.copy_from_slice(digest.as_ref());
    out
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let key = Key::new(HMAC_SHA256, key);
    let tag = sign(&key, data);
    let mut out = [0_u8; 32];
    out.copy_from_slice(tag.as_ref());
    out
}

fn verify_hmac_sha256(key: &[u8], data: &[u8], proof: &[u8; 32]) -> bool {
    let key = Key::new(HMAC_SHA256, key);
    // ring's `verify` compares the recomputed tag in constant time.
    verify(&key, data, proof).is_ok()
}

fn push_u16be(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn push_u32be(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn push_u64be(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn canonical_ticket_bytes(
    format_version: u32,
    host_device_id: &DeviceId,
    host_name: &str,
    relay_hints: &[RelayHint],
    offer_id: &PairOfferId,
    expires_at_unix: u64,
) -> Result<Vec<u8>, PairTicketError> {
    let mut out = Vec::new();
    out.extend_from_slice(TICKET_DOMAIN);
    push_u32be(&mut out, format_version);
    out.extend_from_slice(host_device_id.as_bytes());
    push_len_prefixed_utf8(&mut out, host_name)?;
    push_u8(
        &mut out,
        u8::try_from(relay_hints.len()).map_err(|_| PairTicketError::CanonicalLengthOverflow)?,
    );
    for hint in relay_hints {
        push_len_prefixed_utf8(&mut out, hint.as_str())?;
    }
    out.extend_from_slice(offer_id.as_bytes());
    push_u64be(&mut out, expires_at_unix);
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn canonical_transcript_bytes(
    ticket_digest: &[u8; 32],
    host_device_id: &DeviceId,
    controller_device_id: &DeviceId,
    offer_id: &PairOfferId,
    controller_nonce: &PairNonce,
    host_nonce: &PairNonce,
    controller_name: &str,
    ticket_format_version: u32,
    pair_protocol_version: u32,
    expires_at_unix: u64,
) -> Result<Vec<u8>, PairTicketError> {
    let mut out = Vec::new();
    out.extend_from_slice(TRANSCRIPT_DOMAIN);
    out.extend_from_slice(ticket_digest);
    out.extend_from_slice(host_device_id.as_bytes());
    out.extend_from_slice(controller_device_id.as_bytes());
    out.extend_from_slice(offer_id.as_bytes());
    out.extend_from_slice(controller_nonce.as_bytes());
    out.extend_from_slice(host_nonce.as_bytes());
    push_len_prefixed_utf8(&mut out, controller_name)?;
    push_u32be(&mut out, ticket_format_version);
    push_u32be(&mut out, pair_protocol_version);
    push_u64be(&mut out, expires_at_unix);
    Ok(out)
}

fn push_u8(out: &mut Vec<u8>, value: u8) {
    out.push(value);
}

fn push_len_prefixed_utf8(out: &mut Vec<u8>, text: &str) -> Result<(), PairTicketError> {
    let bytes = text.as_bytes();
    let length =
        u16::try_from(bytes.len()).map_err(|_| PairTicketError::CanonicalLengthOverflow)?;
    push_u16be(out, length);
    out.extend_from_slice(bytes);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_is_redacted_and_length_checked() {
        let secret = PairSecret::from_bytes([0x5a; PAIR_SECRET_BYTES]);
        let debug = format!("{secret:?}");
        let display = format!("{secret}");
        assert!(
            !debug.contains("5a"),
            "Debug must not leak secret bytes: {debug}"
        );
        assert_eq!(display, "[REDACTED]");
        assert_eq!(
            PairSecret::from_slice(&[0; 31]),
            Err(PairSecretError::InvalidLength { actual: 31 })
        );
    }

    #[test]
    fn ttl_and_name_validation_match_the_product_bounds() {
        assert!(validate_pair_ttl(DEFAULT_PAIR_TTL_SECONDS).is_ok());
        assert_eq!(
            validate_pair_ttl(MIN_PAIR_TTL_SECONDS - 1),
            Err(PairTicketError::TtlOutOfRange {
                actual: MIN_PAIR_TTL_SECONDS - 1
            })
        );
        assert_eq!(
            validate_pair_ttl(MAX_PAIR_TTL_SECONDS + 1),
            Err(PairTicketError::TtlOutOfRange {
                actual: MAX_PAIR_TTL_SECONDS + 1
            })
        );
    }

    fn ticket() -> PairTicketFields {
        PairTicketFields::new(
            PAIR_TICKET_FORMAT_VERSION,
            DeviceId::from_array([0x11; 32]),
            "test-host",
            vec![RelayHint::new("https://relay.example.com").expect("valid relay")],
            PairOfferId::from_array([0xaa; 16]),
            1_700_000_000,
        )
        .expect("bounded ticket")
    }

    #[test]
    fn ticket_rejects_duplicate_and_missing_relay_hints() {
        let hint = RelayHint::new("https://relay.example.com").expect("valid relay");
        let other = RelayHint::new("https://other.example.com").expect("valid relay");
        assert!(matches!(
            PairTicketFields::new(
                PAIR_TICKET_FORMAT_VERSION,
                DeviceId::from_array([0x11; 32]),
                "test-host",
                vec![],
                PairOfferId::from_array([0xaa; 16]),
                1,
            ),
            Err(PairTicketError::MissingRelayHint)
        ));
        assert!(matches!(
            PairTicketFields::new(
                PAIR_TICKET_FORMAT_VERSION,
                DeviceId::from_array([0x11; 32]),
                "test-host",
                vec![hint.clone(), hint.clone()],
                PairOfferId::from_array([0xaa; 16]),
                1,
            ),
            Err(PairTicketError::DuplicateRelayHint)
        ));
        let _ = other;
        assert!(matches!(
            PairTicketFields::new(
                2,
                DeviceId::from_array([0x11; 32]),
                "test-host",
                vec![hint],
                PairOfferId::from_array([0xaa; 16]),
                1,
            ),
            Err(PairTicketError::UnsupportedFormatVersion { actual: 2 })
        ));
    }

    #[test]
    fn transcript_rejects_unknown_protocol_version() {
        let ticket = ticket();
        assert!(matches!(
            PairTranscript::new(
                &ticket,
                DeviceId::from_array([0x22; 32]),
                "controller",
                PairNonce::from_array([0x33; 32]),
                PairNonce::from_array([0x44; 32]),
                2,
            ),
            Err(PairTicketError::UnsupportedProtocolVersion { actual: 2 })
        ));
    }
}
