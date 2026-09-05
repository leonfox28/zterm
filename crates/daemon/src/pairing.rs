//! Bounded in-memory ownership for one-time pairing offers.
//!
//! This module deliberately stops before Iroh stream dispatch and SQLite
//! authorization. It owns the proof-before-CAS state transition and returns an
//! opaque consumption permit which the pair-ALPN adapter may either roll back
//! after a pre-commit failure or commit exactly once after the StoreActor has
//! durably authorized the controller.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ring::digest::{SHA256, digest};
use ring::rand::{SecureRandom, SystemRandom};
use zeroize::Zeroizing;
use zterm_core::{
    AuthGeneration, DeviceDisplayName, DeviceId, DomainErrorKind, EphemeralOperationId,
    PAIR_NONCE_BYTES, PAIR_OFFER_ID_BYTES, PAIR_PROTOCOL_VERSION, PAIR_SECRET_BYTES,
    PAIR_TICKET_FORMAT_VERSION, PairAccepted, PairBegin, PairChallenge, PairFingerprint, PairNonce,
    PairOfferId, PairProof, PairSecret, PairTicketError, PairTicketFields, PairTranscript,
    RelayHint, TransportLimits, TransportLimitsError, validate_pair_ttl,
};

use crate::error::DaemonError;

const RANDOM_COLLISION_ATTEMPTS: usize = 8;

/// One wall-clock and monotonic observation used for offer expiry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PairingNow {
    unix_seconds: u64,
    monotonic: Instant,
}

impl PairingNow {
    /// Constructs an injected clock observation.
    #[must_use]
    pub const fn new(unix_seconds: u64, monotonic: Instant) -> Self {
        Self {
            unix_seconds,
            monotonic,
        }
    }

    /// Whole seconds since the Unix epoch.
    #[must_use]
    pub const fn unix_seconds(self) -> u64 {
        self.unix_seconds
    }

    /// Monotonic observation paired with the wall-clock value.
    #[must_use]
    pub const fn monotonic(self) -> Instant {
        self.monotonic
    }
}

/// Opaque failure from an injected pairing clock.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PairingClockError;

impl fmt::Display for PairingClockError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("pairing clock is unavailable")
    }
}

impl std::error::Error for PairingClockError {}

/// Clock boundary used to test expiry without sleeping or changing the OS clock.
pub trait PairingClock: Send + Sync {
    /// Returns one coherent wall-clock and monotonic observation.
    fn now(&self) -> Result<PairingNow, PairingClockError>;
}

/// Production clock using `SystemTime` together with `Instant`.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemPairingClock;

impl PairingClock for SystemPairingClock {
    fn now(&self) -> Result<PairingNow, PairingClockError> {
        let unix_seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| PairingClockError)?
            .as_secs();
        Ok(PairingNow::new(unix_seconds, Instant::now()))
    }
}

/// Opaque failure from the pairing entropy source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PairingEntropyError;

impl fmt::Display for PairingEntropyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("pairing entropy is unavailable")
    }
}

impl std::error::Error for PairingEntropyError {}

/// Entropy boundary used by deterministic state-machine tests.
pub trait PairingEntropy: Send + Sync {
    /// Fills the complete destination with cryptographically secure bytes.
    fn fill(&self, destination: &mut [u8]) -> Result<(), PairingEntropyError>;
}

/// Production entropy backed exclusively by ring's operating-system RNG.
pub struct SystemPairingEntropy {
    random: SystemRandom,
}

impl SystemPairingEntropy {
    /// Constructs the production entropy source.
    #[must_use]
    pub fn new() -> Self {
        Self {
            random: SystemRandom::new(),
        }
    }
}

impl Default for SystemPairingEntropy {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for SystemPairingEntropy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SystemPairingEntropy")
    }
}

impl PairingEntropy for SystemPairingEntropy {
    fn fill(&self, destination: &mut [u8]) -> Result<(), PairingEntropyError> {
        self.random
            .fill(destination)
            .map_err(|_| PairingEntropyError)
    }
}

/// Typed local pairing failure with a separate generic peer projection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PairingError {
    /// The configured transport limits were internally inconsistent.
    InvalidLimits(TransportLimitsError),
    /// Public ticket or transcript fields failed the shared core contract.
    InvalidTicket(PairTicketError),
    /// Ticket fields did not bind the authenticated endpoint or handshake.
    InvalidBinding,
    /// The wall/monotonic clock could not be observed.
    ClockUnavailable,
    /// Expiry or deadline arithmetic could not be represented.
    TimeOverflow,
    /// The operating-system entropy source failed.
    EntropyUnavailable,
    /// The configured live-offer or operation-cell bound was reached.
    ResourceExhausted,
    /// The caller's absolute deadline elapsed.
    DeadlineExceeded,
    /// No live or retained terminal offer matched the identifier.
    OfferNotFound,
    /// The offer expired by either wall clock or monotonic time.
    TicketExpired,
    /// The offer was already committed and consumed.
    TicketConsumed,
    /// Another valid consumer currently owns the pre-commit CAS.
    OfferConsuming,
    /// The controller proof did not authenticate the exact transcript.
    InvalidProof,
    /// An operation ID was reused for another semantic fingerprint.
    OutcomeUnknown,
    /// An opaque challenge or consumption permit did not belong to this state.
    StateConflict,
}

impl PairingError {
    /// Stable, detailed category exposed only to the same-UID local caller.
    #[must_use]
    pub const fn local_kind(self) -> DomainErrorKind {
        match self {
            Self::InvalidLimits(_) | Self::ResourceExhausted => DomainErrorKind::ResourceExhausted,
            Self::ClockUnavailable | Self::EntropyUnavailable => {
                DomainErrorKind::TransportUnavailable
            }
            Self::DeadlineExceeded => DomainErrorKind::DeadlineExceeded,
            Self::TicketExpired => DomainErrorKind::PairTicketExpired,
            Self::TicketConsumed => DomainErrorKind::PairTicketConsumed,
            Self::OfferConsuming | Self::OutcomeUnknown | Self::StateConflict => {
                DomainErrorKind::PairOutcomeUnknown
            }
            Self::InvalidTicket(_)
            | Self::InvalidBinding
            | Self::TimeOverflow
            | Self::OfferNotFound
            | Self::InvalidProof => DomainErrorKind::PairTicketInvalid,
        }
    }

    /// Generic category safe to expose to an unauthenticated pair peer.
    #[must_use]
    pub const fn peer_kind(self) -> DomainErrorKind {
        match self {
            Self::InvalidLimits(_) | Self::ResourceExhausted => DomainErrorKind::ResourceExhausted,
            Self::ClockUnavailable | Self::EntropyUnavailable => {
                DomainErrorKind::TransportUnavailable
            }
            Self::DeadlineExceeded => DomainErrorKind::DeadlineExceeded,
            _ => DomainErrorKind::PairTicketInvalid,
        }
    }

    /// Generic daemon error which does not distinguish offer state or proof failure.
    #[must_use]
    pub fn peer_error(self) -> DaemonError {
        let detail = match self.peer_kind() {
            DomainErrorKind::ResourceExhausted => "pairing service is overloaded",
            DomainErrorKind::TransportUnavailable => "pairing service is unavailable",
            DomainErrorKind::DeadlineExceeded => "pairing handshake deadline elapsed",
            _ => "pairing request was rejected",
        };
        DaemonError::new(self.peer_kind(), detail)
    }
}

impl fmt::Display for PairingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLimits(error) => error.fmt(formatter),
            Self::InvalidTicket(error) => error.fmt(formatter),
            Self::InvalidBinding => formatter.write_str("pairing handshake binding is invalid"),
            Self::ClockUnavailable => formatter.write_str("pairing clock is unavailable"),
            Self::TimeOverflow => formatter.write_str("pairing time arithmetic overflowed"),
            Self::EntropyUnavailable => formatter.write_str("pairing entropy is unavailable"),
            Self::ResourceExhausted => formatter.write_str("pairing offer capacity is exhausted"),
            Self::DeadlineExceeded => formatter.write_str("pairing operation deadline elapsed"),
            Self::OfferNotFound => formatter.write_str("pairing offer is unavailable"),
            Self::TicketExpired => formatter.write_str("pairing ticket has expired"),
            Self::TicketConsumed => formatter.write_str("pairing ticket was already consumed"),
            Self::OfferConsuming => {
                formatter.write_str("pairing ticket already has a pre-commit consumer")
            }
            Self::InvalidProof => formatter.write_str("pairing proof was rejected"),
            Self::OutcomeUnknown => formatter.write_str("pairing operation outcome is unknown"),
            Self::StateConflict => formatter.write_str("pairing state changed unexpectedly"),
        }
    }
}

impl std::error::Error for PairingError {}

impl From<PairingError> for DaemonError {
    fn from(error: PairingError) -> Self {
        Self::new(error.local_kind(), error.to_string())
    }
}

/// Validated semantic input for one local pair-create mutation.
#[derive(Clone, Debug)]
pub struct PairOfferRequest {
    operation_id: EphemeralOperationId,
    fingerprint: PairFingerprint,
    host_name: DeviceDisplayName,
    relay_hints: Vec<RelayHint>,
    ttl_seconds: u64,
}

impl PairOfferRequest {
    /// Validates a request before it can allocate operation replay state.
    pub fn new(
        operation_id: EphemeralOperationId,
        fingerprint: PairFingerprint,
        host_name: DeviceDisplayName,
        relay_hints: Vec<RelayHint>,
        ttl_seconds: u64,
    ) -> Result<Self, PairingError> {
        validate_pair_ttl(ttl_seconds).map_err(PairingError::InvalidTicket)?;
        // Reuse the core owner for ordered relay-list and display-name policy.
        PairTicketFields::new(
            PAIR_TICKET_FORMAT_VERSION,
            DeviceId::from_array([0; 32]),
            host_name.as_str(),
            relay_hints.clone(),
            PairOfferId::from_array([0; PAIR_OFFER_ID_BYTES]),
            1,
        )
        .map_err(PairingError::InvalidTicket)?;
        Ok(Self {
            operation_id,
            fingerprint,
            host_name,
            relay_hints,
            ttl_seconds,
        })
    }

    /// Ephemeral local mutation identity.
    #[must_use]
    pub const fn operation_id(&self) -> EphemeralOperationId {
        self.operation_id
    }

    /// Redacted semantic fingerprint used for join/replay conflict detection.
    #[must_use]
    pub fn fingerprint(&self) -> &PairFingerprint {
        &self.fingerprint
    }
}

/// Redacted, zeroizing encoded bearer ticket.
#[derive(Eq, PartialEq)]
pub struct PairTicketText(Zeroizing<String>);

impl PairTicketText {
    fn new(text: String) -> Self {
        Self(Zeroizing::new(text))
    }

    /// Takes ownership of a ticket returned by the trusted local daemon after
    /// revalidating its bounded text envelope. The parsed secret and decoded
    /// protobuf buffer are dropped through their zeroizing owners before this
    /// function returns; only the intended bearer text remains.
    pub fn from_local_response(text: String) -> Result<Self, PairingError> {
        let text = Zeroizing::new(text);
        if text.len() > zterm_core::MAX_TICKET_TEXT_BYTES {
            return Err(PairingError::ResourceExhausted);
        }
        let _ = zterm_proto::decode_pair_ticket(&text).map_err(|_| PairingError::InvalidBinding)?;
        Ok(Self(text))
    }

    /// Borrows the bearer text only at the explicit IPC/network boundary.
    #[must_use]
    pub fn expose(&self) -> &str {
        self.0.as_str()
    }
}

impl Clone for PairTicketText {
    fn clone(&self) -> Self {
        Self::new(self.0.as_str().to_owned())
    }
}

impl fmt::Debug for PairTicketText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PairTicketText([REDACTED])")
    }
}

impl fmt::Display for PairTicketText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

/// Exact replayable result of one local pair-create mutation.
#[derive(Clone, Eq, PartialEq)]
pub struct PairOfferCreated {
    fields: PairTicketFields,
    ticket: PairTicketText,
}

impl PairOfferCreated {
    /// Public secret-free ticket fields.
    #[must_use]
    pub fn fields(&self) -> &PairTicketFields {
        &self.fields
    }

    /// Zeroizing bearer text returned by the local IPC adapter.
    #[must_use]
    pub fn ticket(&self) -> &PairTicketText {
        &self.ticket
    }
}

impl fmt::Debug for PairOfferCreated {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PairOfferCreated")
            .field("fields", &self.fields)
            .field("ticket", &"[REDACTED]")
            .finish()
    }
}

/// Observable offer state without verifier, ticket, proof, or transcript bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PairOfferState {
    /// Offer may accept a valid proof.
    Ready,
    /// One valid consumer owns the pre-commit CAS.
    Consuming,
    /// Durable authorization committed and the offer cannot reopen.
    Consumed,
    /// Either expiry clock reached its deadline.
    Expired,
}

/// Bounded aggregate observation of PairingManager-owned memory.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PairingSnapshot {
    /// Ready live offers.
    pub ready_offers: usize,
    /// Offers held by one pre-commit consumer.
    pub consuming_offers: usize,
    /// Exact ticket replay operation cells.
    pub operation_cells: usize,
    /// Ticket-free retained operation outcomes used to reject recent ID reuse.
    pub retired_operation_tombstones: usize,
    /// Retained bounded consumed identifiers.
    pub consumed_tombstones: usize,
    /// Retained bounded expired identifiers.
    pub expired_tombstones: usize,
}

/// Host challenge and exact core transcript for one attempted consumer.
#[derive(Clone)]
pub struct PreparedPairChallenge {
    owner: Weak<PairingInner>,
    offer_id: PairOfferId,
    controller_device_id: DeviceId,
    controller_name: DeviceDisplayName,
    challenge: PairChallenge,
    transcript: PairTranscript,
    ticket_digest: [u8; 32],
    transcript_digest: [u8; 32],
}

impl PreparedPairChallenge {
    /// Wire-ready challenge validated by zterm-core.
    #[must_use]
    pub const fn challenge(&self) -> &PairChallenge {
        &self.challenge
    }

    /// Exact canonical transcript used for proof construction and verification.
    #[must_use]
    pub const fn transcript(&self) -> &PairTranscript {
        &self.transcript
    }

    /// TLS-authenticated controller identity bound into the transcript.
    #[must_use]
    pub const fn controller_device_id(&self) -> DeviceId {
        self.controller_device_id
    }
}

impl fmt::Debug for PreparedPairChallenge {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedPairChallenge")
            .field("offer_id", &self.offer_id)
            .field("controller_device_id", &self.controller_device_id)
            .field("controller_name", &self.controller_name)
            .field("challenge", &"[REDACTED]")
            .field("transcript", &"[REDACTED]")
            .finish()
    }
}

/// Opaque proof-before-CAS ownership passed to the durable authorize step.
///
/// This permit must be explicitly rolled back only while durable authorization
/// is known not to have started, or committed after the StoreActor returns its
/// exact success. Dropping it deliberately leaves the offer `Consuming`: an
/// ambiguous store outcome must fail closed instead of reopening a ticket that
/// may already have authorized the controller.
#[must_use = "explicitly rollback before durable work or commit after exact StoreActor success"]
pub struct PairConsumption {
    owner: Arc<PairingInner>,
    offer_id: PairOfferId,
    controller_device_id: DeviceId,
    controller_name: DeviceDisplayName,
    transcript_digest: [u8; 32],
    transcript: PairTranscript,
}

impl PairConsumption {
    /// Controller which won the CAS and may be durably authorized.
    #[must_use]
    pub const fn controller_device_id(&self) -> DeviceId {
        self.controller_device_id
    }

    /// Validated controller display name to persist with authorization.
    #[must_use]
    pub fn controller_name(&self) -> &DeviceDisplayName {
        &self.controller_name
    }

    /// Exact transcript later bound into host confirmation.
    #[must_use]
    pub const fn transcript(&self) -> &PairTranscript {
        &self.transcript
    }
}

impl fmt::Debug for PairConsumption {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PairConsumption")
            .field("offer_id", &self.offer_id)
            .field("controller_device_id", &self.controller_device_id)
            .field("controller_name", &self.controller_name)
            .field("transcript", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

/// Redacted, zeroizing confirmation returned after durable authorization.
pub struct PairCommitResult {
    generation: AuthGeneration,
    host_confirmation_proof: Zeroizing<[u8; 32]>,
    host_diagnostic_version: DeviceDisplayName,
}

impl PairCommitResult {
    /// Resulting durable authorization generation.
    #[must_use]
    pub const fn authorization_generation(&self) -> AuthGeneration {
        self.generation
    }

    /// Borrows the transcript-bound confirmation only at the wire boundary.
    #[must_use]
    pub fn host_confirmation_proof(&self) -> &[u8; 32] {
        &self.host_confirmation_proof
    }

    /// Produces the shared core value consumed by the protobuf adapter.
    ///
    /// The returned short-lived value should be encoded immediately; the
    /// manager-owned copy remains zeroizing and redacted.
    pub fn pair_accepted(&self) -> Result<PairAccepted, PairingError> {
        PairAccepted::new(
            self.generation,
            *self.host_confirmation_proof,
            self.host_diagnostic_version.as_str(),
        )
        .map_err(PairingError::InvalidTicket)
    }
}

impl fmt::Debug for PairCommitResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PairCommitResult")
            .field("authorization_generation", &self.generation)
            .field("host_confirmation_proof", &"[REDACTED]")
            .field("host_diagnostic_version", &self.host_diagnostic_version)
            .finish()
    }
}

/// Builds the exact controller-side transcript after checking TLS/ticket bindings.
pub fn controller_transcript(
    ticket: &PairTicketFields,
    authenticated_host: DeviceId,
    controller_device_id: DeviceId,
    begin: &PairBegin,
    challenge: &PairChallenge,
) -> Result<PairTranscript, PairingError> {
    if ticket.host_device_id() != authenticated_host
        || ticket.offer_id() != begin.offer_id()
        || ticket.expires_at_unix() != challenge.ticket_expiry_unix()
        || begin.pair_protocol_version() != challenge.selected_version()
    {
        return Err(PairingError::InvalidBinding);
    }
    PairTranscript::new(
        ticket,
        controller_device_id,
        begin.controller_name(),
        begin.controller_nonce(),
        challenge.host_nonce(),
        challenge.selected_version(),
    )
    .map_err(PairingError::InvalidTicket)
}

/// Cloneable bounded owner for pairing offers and local create replay cells.
#[derive(Clone)]
pub struct PairingManager {
    inner: Arc<PairingInner>,
}

impl PairingManager {
    /// Constructs the production manager with SystemTime/Instant and SystemRandom.
    pub fn new(host_device_id: DeviceId, limits: TransportLimits) -> Result<Self, PairingError> {
        Self::with_dependencies(
            host_device_id,
            limits,
            Arc::new(SystemPairingClock),
            Arc::new(SystemPairingEntropy::new()),
        )
    }

    /// Constructs a manager with injectable pure clock and entropy boundaries.
    pub fn with_dependencies(
        host_device_id: DeviceId,
        limits: TransportLimits,
        clock: Arc<dyn PairingClock>,
        entropy: Arc<dyn PairingEntropy>,
    ) -> Result<Self, PairingError> {
        limits.validate().map_err(PairingError::InvalidLimits)?;
        Ok(Self {
            inner: Arc::new(PairingInner {
                host_device_id,
                max_live_offers: limits.max_live_pair_offers,
                max_ticket_text_bytes: limits.max_ticket_text_bytes,
                max_relay_hints: limits.max_relay_hints,
                max_relay_url_bytes: limits.max_relay_url_bytes,
                operation_wait: limits.pairing_total_deadline,
                clock,
                entropy,
                state: Mutex::new(ManagerState::default()),
            }),
        })
    }

    /// Host identity embedded into every ticket and transcript.
    #[must_use]
    pub fn host_device_id(&self) -> DeviceId {
        self.inner.host_device_id
    }

    /// Creates or joins/replays one local ticket mutation under a bounded deadline.
    pub fn create_offer(
        &self,
        request: PairOfferRequest,
    ) -> Result<PairOfferCreated, PairingError> {
        let deadline = default_deadline(self.inner.operation_wait)?;
        self.create_offer_until(request, deadline)
    }

    /// Creates or joins/replays one local ticket mutation under an absolute deadline.
    pub fn create_offer_until(
        &self,
        request: PairOfferRequest,
        deadline: Instant,
    ) -> Result<PairOfferCreated, PairingError> {
        check_deadline(deadline)?;
        self.inner.validate_request_limits(&request)?;
        let now = self.inner.now()?;
        let expires_at_unix = now
            .unix_seconds
            .checked_add(request.ttl_seconds)
            .ok_or(PairingError::TimeOverflow)?;
        let monotonic_deadline = now
            .monotonic
            .checked_add(Duration::from_secs(request.ttl_seconds))
            .ok_or(PairingError::TimeOverflow)?;

        let cell = {
            let mut state = state_lock(&self.inner.state);
            self.inner.expire_ready_locked(&mut state, now)?;
            if let Some(cell) = state.operations.get(&request.operation_id) {
                if cell.fingerprint != request.fingerprint {
                    return Err(PairingError::OutcomeUnknown);
                }
                let cell = Arc::clone(cell);
                drop(state);
                return cell.wait_until(deadline);
            }
            if let Some(retired) = state.retired_operations.get(&request.operation_id) {
                return if retired.fingerprint == request.fingerprint {
                    Err(retired.error)
                } else {
                    Err(PairingError::OutcomeUnknown)
                };
            }
            if state.offers.len() >= self.inner.max_live_offers
                || state.operations.len() >= self.inner.max_live_offers
            {
                return Err(PairingError::ResourceExhausted);
            }
            let cell = Arc::new(OperationCell::new(request.fingerprint.clone()));
            state
                .operations
                .insert(request.operation_id, Arc::clone(&cell));
            cell
        };

        let result =
            self.inner
                .execute_create(&request, expires_at_unix, monotonic_deadline, deadline);
        match &result {
            Ok(_) => tracing::info!(
                component = "pairing",
                operation = "offer_created",
                ttl_seconds = request.ttl_seconds,
                "Pairing ticket created"
            ),
            Err(error) => tracing::warn!(
                component = "pairing",
                operation = "offer_failed",
                reason = DaemonError::from(*error).kind().code(),
                "Pairing ticket creation failed"
            ),
        }
        cell.complete(result.clone());
        if let Err(error) = result {
            let mut state = state_lock(&self.inner.state);
            if state
                .operations
                .get(&request.operation_id)
                .is_some_and(|current| Arc::ptr_eq(current, &cell))
            {
                self.inner.retire_operation_locked(
                    &mut state,
                    request.operation_id,
                    request.fingerprint,
                    error,
                );
            }
        }
        result
    }

    /// Generates one controller or host nonce with SystemRandom in production.
    pub fn random_nonce(&self) -> Result<PairNonce, PairingError> {
        let deadline = default_deadline(self.inner.operation_wait)?;
        self.random_nonce_until(deadline)
    }

    /// Generates one nonce under an absolute caller deadline.
    pub fn random_nonce_until(&self, deadline: Instant) -> Result<PairNonce, PairingError> {
        check_deadline(deadline)?;
        let mut bytes = [0_u8; PAIR_NONCE_BYTES];
        self.inner
            .entropy
            .fill(&mut bytes)
            .map_err(|_| PairingError::EntropyUnavailable)?;
        check_deadline(deadline)?;
        Ok(PairNonce::from_array(bytes))
    }

    /// Prepares the host challenge without consuming the offer.
    pub fn prepare_challenge(
        &self,
        controller_device_id: DeviceId,
        begin: &PairBegin,
    ) -> Result<PreparedPairChallenge, PairingError> {
        let deadline = default_deadline(self.inner.operation_wait)?;
        self.prepare_challenge_until(controller_device_id, begin, deadline)
    }

    /// Prepares the host challenge under an absolute caller deadline.
    pub fn prepare_challenge_until(
        &self,
        controller_device_id: DeviceId,
        begin: &PairBegin,
        deadline: Instant,
    ) -> Result<PreparedPairChallenge, PairingError> {
        check_deadline(deadline)?;
        self.inner.ready_fields(begin.offer_id())?;
        let host_nonce = self.random_nonce_until(deadline)?;
        let fields = self.inner.ready_fields(begin.offer_id())?;
        let challenge =
            PairChallenge::new(host_nonce, PAIR_PROTOCOL_VERSION, fields.expires_at_unix())
                .map_err(PairingError::InvalidTicket)?;
        let transcript = controller_transcript(
            &fields,
            self.inner.host_device_id,
            controller_device_id,
            begin,
            &challenge,
        )?;
        let ticket_digest = fields.ticket_digest();
        let transcript_digest = sha256(transcript.canonical_bytes());
        Ok(PreparedPairChallenge {
            owner: Arc::downgrade(&self.inner),
            offer_id: begin.offer_id(),
            controller_device_id,
            controller_name: DeviceDisplayName::new(begin.controller_name())
                .map_err(|_| PairingError::InvalidBinding)?,
            challenge,
            transcript,
            ticket_digest,
            transcript_digest,
        })
    }

    /// Verifies proof first, then atomically transitions Ready to Consuming.
    pub fn try_consume(
        &self,
        challenge: PreparedPairChallenge,
        proof: &PairProof,
    ) -> Result<PairConsumption, PairingError> {
        let deadline = default_deadline(self.inner.operation_wait)?;
        self.try_consume_until(challenge, proof, deadline)
    }

    /// Verifies proof and performs the CAS under an absolute caller deadline.
    pub fn try_consume_until(
        &self,
        challenge: PreparedPairChallenge,
        proof: &PairProof,
        deadline: Instant,
    ) -> Result<PairConsumption, PairingError> {
        check_deadline(deadline)?;
        let Some(owner) = challenge.owner.upgrade() else {
            return Err(PairingError::StateConflict);
        };
        if !Arc::ptr_eq(&owner, &self.inner) {
            return Err(PairingError::StateConflict);
        }
        let now = self.inner.now()?;
        let mut state = state_lock(&self.inner.state);
        self.inner.expire_ready_locked(&mut state, now)?;
        let Some(record) = state.offers.get_mut(&challenge.offer_id) else {
            return Err(self.inner.offer_terminal_error(&state, challenge.offer_id));
        };
        if record.fields.ticket_digest() != challenge.ticket_digest {
            return Err(PairingError::StateConflict);
        }
        match record.state {
            LiveOfferState::Ready => {
                if !challenge
                    .transcript
                    .verify_controller_proof(&record.offer_key, proof.as_bytes())
                {
                    return Err(PairingError::InvalidProof);
                }
                record.state = LiveOfferState::Consuming {
                    controller_device_id: challenge.controller_device_id,
                    transcript_digest: challenge.transcript_digest,
                };
            }
            LiveOfferState::Consuming { .. } => return Err(PairingError::OfferConsuming),
        }
        drop(state);
        Ok(PairConsumption {
            owner,
            offer_id: challenge.offer_id,
            controller_device_id: challenge.controller_device_id,
            controller_name: challenge.controller_name,
            transcript_digest: challenge.transcript_digest,
            transcript: challenge.transcript,
        })
    }

    /// Rolls a pre-commit failure back to Ready, or Expired if either clock elapsed.
    pub fn rollback(&self, consumption: PairConsumption) -> Result<PairOfferState, PairingError> {
        if !Arc::ptr_eq(&self.inner, &consumption.owner) {
            return Err(PairingError::StateConflict);
        }
        self.inner.rollback_token(
            consumption.offer_id,
            consumption.controller_device_id,
            consumption.transcript_digest,
        )
    }

    /// Marks a durably authorized consumer as consumed exactly once.
    ///
    /// This method is intentionally called only after the StoreActor commit.
    /// It permits a commit that finishes just after ticket expiry: reopening or
    /// discarding an already durable authorization would be less safe.
    pub fn commit(
        &self,
        consumption: PairConsumption,
        generation: AuthGeneration,
        host_diagnostic_version: &DeviceDisplayName,
    ) -> Result<PairCommitResult, PairingError> {
        if !Arc::ptr_eq(&self.inner, &consumption.owner) {
            return Err(PairingError::StateConflict);
        }
        if generation == AuthGeneration::ZERO {
            return Err(PairingError::InvalidTicket(
                PairTicketError::ZeroAuthorizationGeneration,
            ));
        }
        self.inner.commit_token(
            consumption.offer_id,
            consumption.controller_device_id,
            consumption.transcript_digest,
            &consumption.transcript,
            generation,
            host_diagnostic_version,
        )
    }

    /// Returns one offer's public state after applying expiry cleanup.
    pub fn offer_state(&self, offer_id: PairOfferId) -> Result<PairOfferState, PairingError> {
        let now = self.inner.now()?;
        let mut state = state_lock(&self.inner.state);
        self.inner.expire_ready_locked(&mut state, now)?;
        if let Some(record) = state.offers.get(&offer_id) {
            return Ok(match record.state {
                LiveOfferState::Ready => PairOfferState::Ready,
                LiveOfferState::Consuming { .. } => PairOfferState::Consuming,
            });
        }
        match state.retired_offers.get(&offer_id).map(|entry| entry.error) {
            Some(PairingError::TicketConsumed) => Ok(PairOfferState::Consumed),
            Some(PairingError::TicketExpired) => Ok(PairOfferState::Expired),
            Some(error) => Err(error),
            None => Err(PairingError::OfferNotFound),
        }
    }

    /// Removes every Ready offer expired by wall or monotonic time.
    pub fn prune_expired(&self) -> Result<usize, PairingError> {
        let now = self.inner.now()?;
        self.inner
            .expire_ready_locked(&mut state_lock(&self.inner.state), now)
    }

    /// Returns bounded counts only; no ticket, verifier, route, or transcript.
    pub fn snapshot(&self) -> Result<PairingSnapshot, PairingError> {
        let now = self.inner.now()?;
        let mut state = state_lock(&self.inner.state);
        self.inner.expire_ready_locked(&mut state, now)?;
        let mut snapshot = PairingSnapshot {
            operation_cells: state.operations.len(),
            retired_operation_tombstones: state.retired_operations.len(),
            ..PairingSnapshot::default()
        };
        for record in state.offers.values() {
            match record.state {
                LiveOfferState::Ready => {
                    snapshot.ready_offers = snapshot
                        .ready_offers
                        .checked_add(1)
                        .ok_or(PairingError::ResourceExhausted)?;
                }
                LiveOfferState::Consuming { .. } => {
                    snapshot.consuming_offers = snapshot
                        .consuming_offers
                        .checked_add(1)
                        .ok_or(PairingError::ResourceExhausted)?;
                }
            }
        }
        for retired in state.retired_offers.values() {
            match retired.error {
                PairingError::TicketConsumed => {
                    snapshot.consumed_tombstones = snapshot
                        .consumed_tombstones
                        .checked_add(1)
                        .ok_or(PairingError::ResourceExhausted)?;
                }
                PairingError::TicketExpired => {
                    snapshot.expired_tombstones = snapshot
                        .expired_tombstones
                        .checked_add(1)
                        .ok_or(PairingError::ResourceExhausted)?;
                }
                _ => {}
            }
        }
        Ok(snapshot)
    }
}

impl fmt::Debug for PairingManager {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PairingManager")
            .field("host_device_id", &self.inner.host_device_id)
            .field("state", &"[REDACTED]")
            .finish()
    }
}

struct PairingInner {
    host_device_id: DeviceId,
    max_live_offers: usize,
    max_ticket_text_bytes: usize,
    max_relay_hints: usize,
    max_relay_url_bytes: usize,
    operation_wait: Duration,
    clock: Arc<dyn PairingClock>,
    entropy: Arc<dyn PairingEntropy>,
    state: Mutex<ManagerState>,
}

impl PairingInner {
    fn now(&self) -> Result<PairingNow, PairingError> {
        self.clock.now().map_err(|_| PairingError::ClockUnavailable)
    }

    fn validate_request_limits(&self, request: &PairOfferRequest) -> Result<(), PairingError> {
        if request.relay_hints.len() > self.max_relay_hints
            || request
                .relay_hints
                .iter()
                .any(|hint| hint.as_str().len() > self.max_relay_url_bytes)
        {
            return Err(PairingError::ResourceExhausted);
        }
        Ok(())
    }

    fn execute_create(
        &self,
        request: &PairOfferRequest,
        expires_at_unix: u64,
        monotonic_deadline: Instant,
        deadline: Instant,
    ) -> Result<PairOfferCreated, PairingError> {
        for _ in 0..RANDOM_COLLISION_ATTEMPTS {
            check_deadline(deadline)?;
            let mut offer_id_bytes = [0_u8; PAIR_OFFER_ID_BYTES];
            self.entropy
                .fill(&mut offer_id_bytes)
                .map_err(|_| PairingError::EntropyUnavailable)?;
            let offer_id = PairOfferId::from_array(offer_id_bytes);

            {
                let state = state_lock(&self.state);
                if state.offers.contains_key(&offer_id)
                    || state.retired_offers.contains_key(&offer_id)
                {
                    continue;
                }
            }

            let mut secret_bytes = Zeroizing::new([0_u8; PAIR_SECRET_BYTES]);
            self.entropy
                .fill(secret_bytes.as_mut())
                .map_err(|_| PairingError::EntropyUnavailable)?;
            check_deadline(deadline)?;
            let secret = PairSecret::from_bytes(*secret_bytes);
            let fields = PairTicketFields::new(
                PAIR_TICKET_FORMAT_VERSION,
                self.host_device_id,
                request.host_name.as_str(),
                request.relay_hints.clone(),
                offer_id,
                expires_at_unix,
            )
            .map_err(PairingError::InvalidTicket)?;
            let offer_key = Zeroizing::new(fields.offer_key(&secret));
            let ticket = PairTicketText::new(zterm_proto::encode_pair_ticket(&fields, &secret));
            if ticket.expose().len() > self.max_ticket_text_bytes {
                return Err(PairingError::ResourceExhausted);
            }
            let now = self.now()?;
            if expired(&fields, monotonic_deadline, now) {
                return Err(PairingError::TicketExpired);
            }
            let created = PairOfferCreated {
                fields: fields.clone(),
                ticket,
            };
            let mut state = state_lock(&self.state);
            self.expire_ready_locked(&mut state, now)?;
            if state.offers.contains_key(&offer_id) || state.retired_offers.contains_key(&offer_id)
            {
                continue;
            }
            if state.offers.len() >= self.max_live_offers {
                return Err(PairingError::ResourceExhausted);
            }
            state.offers.insert(
                offer_id,
                OfferRecord {
                    fields,
                    offer_key,
                    monotonic_deadline,
                    operation_id: request.operation_id,
                    fingerprint: request.fingerprint.clone(),
                    state: LiveOfferState::Ready,
                },
            );
            return Ok(created);
        }
        Err(PairingError::ResourceExhausted)
    }

    fn ready_fields(&self, offer_id: PairOfferId) -> Result<PairTicketFields, PairingError> {
        let now = self.now()?;
        let mut state = state_lock(&self.state);
        self.expire_ready_locked(&mut state, now)?;
        match state.offers.get(&offer_id) {
            Some(OfferRecord {
                state: LiveOfferState::Ready,
                fields,
                ..
            }) => Ok(fields.clone()),
            Some(OfferRecord {
                state: LiveOfferState::Consuming { .. },
                ..
            }) => Err(PairingError::OfferConsuming),
            None => Err(self.offer_terminal_error(&state, offer_id)),
        }
    }

    fn rollback_token(
        &self,
        offer_id: PairOfferId,
        controller_device_id: DeviceId,
        transcript_digest: [u8; 32],
    ) -> Result<PairOfferState, PairingError> {
        let now = self.clock.now().ok();
        let mut state = state_lock(&self.state);
        let Some(record) = state.offers.get_mut(&offer_id) else {
            return match state.retired_offers.get(&offer_id).map(|entry| entry.error) {
                Some(PairingError::TicketExpired) => Ok(PairOfferState::Expired),
                Some(PairingError::TicketConsumed) => Ok(PairOfferState::Consumed),
                Some(error) => Err(error),
                None => Err(PairingError::StateConflict),
            };
        };
        if !matches!(
            record.state,
            LiveOfferState::Consuming {
                controller_device_id: current_controller,
                transcript_digest: current_digest,
            } if current_controller == controller_device_id && current_digest == transcript_digest
        ) {
            return Err(PairingError::StateConflict);
        }
        if now.is_some_and(|now| expired(&record.fields, record.monotonic_deadline, now)) {
            let Some(record) = state.offers.remove(&offer_id) else {
                return Err(PairingError::StateConflict);
            };
            self.retire_offer_record_locked(
                &mut state,
                offer_id,
                record,
                PairingError::TicketExpired,
            );
            return Ok(PairOfferState::Expired);
        }
        record.state = LiveOfferState::Ready;
        Ok(PairOfferState::Ready)
    }

    #[allow(clippy::too_many_arguments)]
    fn commit_token(
        &self,
        offer_id: PairOfferId,
        controller_device_id: DeviceId,
        transcript_digest: [u8; 32],
        transcript: &PairTranscript,
        generation: AuthGeneration,
        host_diagnostic_version: &DeviceDisplayName,
    ) -> Result<PairCommitResult, PairingError> {
        let mut state = state_lock(&self.state);
        let confirmation = {
            let Some(record) = state.offers.get(&offer_id) else {
                return Err(self.offer_terminal_error(&state, offer_id));
            };
            if !matches!(
                record.state,
                LiveOfferState::Consuming {
                    controller_device_id: current_controller,
                    transcript_digest: current_digest,
                } if current_controller == controller_device_id && current_digest == transcript_digest
            ) {
                return Err(PairingError::StateConflict);
            }
            if sha256(transcript.canonical_bytes()) != transcript_digest {
                return Err(PairingError::StateConflict);
            }
            PairCommitResult {
                generation,
                host_confirmation_proof: Zeroizing::new(
                    transcript.host_confirmation(&record.offer_key, generation.get()),
                ),
                host_diagnostic_version: host_diagnostic_version.clone(),
            }
        };
        let Some(record) = state.offers.remove(&offer_id) else {
            return Err(PairingError::StateConflict);
        };
        self.retire_offer_record_locked(&mut state, offer_id, record, PairingError::TicketConsumed);
        Ok(confirmation)
    }

    fn expire_ready_locked(
        &self,
        state: &mut ManagerState,
        now: PairingNow,
    ) -> Result<usize, PairingError> {
        let expired_ids = state
            .offers
            .iter()
            .filter_map(|(offer_id, record)| {
                (matches!(record.state, LiveOfferState::Ready)
                    && expired(&record.fields, record.monotonic_deadline, now))
                .then_some(*offer_id)
            })
            .collect::<Vec<_>>();
        let mut removed = 0_usize;
        for offer_id in expired_ids {
            let Some(record) = state.offers.remove(&offer_id) else {
                continue;
            };
            self.retire_offer_record_locked(state, offer_id, record, PairingError::TicketExpired);
            removed = removed
                .checked_add(1)
                .ok_or(PairingError::ResourceExhausted)?;
        }
        Ok(removed)
    }

    fn retire_offer_record_locked(
        &self,
        state: &mut ManagerState,
        offer_id: PairOfferId,
        record: OfferRecord,
        error: PairingError,
    ) {
        self.retire_operation_locked(state, record.operation_id, record.fingerprint, error);
        insert_retired_offer(state, self.max_live_offers, offer_id, error);
        // `record` drops here, zeroizing the derived verifier.
    }

    fn retire_operation_locked(
        &self,
        state: &mut ManagerState,
        operation_id: EphemeralOperationId,
        fingerprint: PairFingerprint,
        error: PairingError,
    ) {
        if let Some(cell) = state.operations.remove(&operation_id) {
            cell.retire(error);
        }
        insert_retired_operation(
            state,
            self.max_live_offers,
            operation_id,
            RetiredOperation { fingerprint, error },
        );
    }

    fn offer_terminal_error(&self, state: &ManagerState, offer_id: PairOfferId) -> PairingError {
        state
            .retired_offers
            .get(&offer_id)
            .map_or(PairingError::OfferNotFound, |entry| entry.error)
    }
}

#[derive(Default)]
struct ManagerState {
    offers: BTreeMap<PairOfferId, OfferRecord>,
    operations: BTreeMap<EphemeralOperationId, Arc<OperationCell>>,
    retired_offers: BTreeMap<PairOfferId, RetiredOffer>,
    retired_offer_order: VecDeque<PairOfferId>,
    retired_operations: BTreeMap<EphemeralOperationId, RetiredOperation>,
    retired_operation_order: VecDeque<EphemeralOperationId>,
}

struct OfferRecord {
    fields: PairTicketFields,
    offer_key: Zeroizing<[u8; 32]>,
    monotonic_deadline: Instant,
    operation_id: EphemeralOperationId,
    fingerprint: PairFingerprint,
    state: LiveOfferState,
}

#[derive(Clone, Copy)]
enum LiveOfferState {
    Ready,
    Consuming {
        controller_device_id: DeviceId,
        transcript_digest: [u8; 32],
    },
}

#[derive(Clone, Copy)]
struct RetiredOffer {
    error: PairingError,
}

struct RetiredOperation {
    fingerprint: PairFingerprint,
    error: PairingError,
}

struct OperationCell {
    fingerprint: PairFingerprint,
    outcome: Mutex<Option<Result<PairOfferCreated, PairingError>>>,
    changed: Condvar,
}

impl OperationCell {
    fn new(fingerprint: PairFingerprint) -> Self {
        Self {
            fingerprint,
            outcome: Mutex::new(None),
            changed: Condvar::new(),
        }
    }

    fn complete(&self, outcome: Result<PairOfferCreated, PairingError>) {
        let mut current = cell_lock(&self.outcome);
        if current.is_none() {
            *current = Some(outcome);
            self.changed.notify_all();
        }
    }

    fn retire(&self, error: PairingError) {
        let mut current = cell_lock(&self.outcome);
        *current = Some(Err(error));
        self.changed.notify_all();
    }

    fn wait_until(&self, deadline: Instant) -> Result<PairOfferCreated, PairingError> {
        let mut outcome = cell_lock(&self.outcome);
        loop {
            if let Some(outcome) = &*outcome {
                return outcome.clone();
            }
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or(PairingError::DeadlineExceeded)?;
            let (next, timed) = self
                .changed
                .wait_timeout(outcome, remaining)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            outcome = next;
            if timed.timed_out() && outcome.is_none() {
                return Err(PairingError::DeadlineExceeded);
            }
        }
    }
}

fn insert_retired_offer(
    state: &mut ManagerState,
    maximum: usize,
    offer_id: PairOfferId,
    error: PairingError,
) {
    state
        .retired_offer_order
        .retain(|current| *current != offer_id);
    state
        .retired_offers
        .insert(offer_id, RetiredOffer { error });
    state.retired_offer_order.push_back(offer_id);
    while state.retired_offers.len() > maximum {
        let Some(oldest) = state.retired_offer_order.pop_front() else {
            break;
        };
        state.retired_offers.remove(&oldest);
    }
}

fn insert_retired_operation(
    state: &mut ManagerState,
    maximum: usize,
    operation_id: EphemeralOperationId,
    retired: RetiredOperation,
) {
    state
        .retired_operation_order
        .retain(|current| *current != operation_id);
    state.retired_operations.insert(operation_id, retired);
    state.retired_operation_order.push_back(operation_id);
    while state.retired_operations.len() > maximum {
        let Some(oldest) = state.retired_operation_order.pop_front() else {
            break;
        };
        state.retired_operations.remove(&oldest);
    }
}

fn expired(fields: &PairTicketFields, monotonic_deadline: Instant, now: PairingNow) -> bool {
    fields.is_expired(now.unix_seconds) || now.monotonic >= monotonic_deadline
}

fn check_deadline(deadline: Instant) -> Result<(), PairingError> {
    if Instant::now() >= deadline {
        Err(PairingError::DeadlineExceeded)
    } else {
        Ok(())
    }
}

fn default_deadline(duration: Duration) -> Result<Instant, PairingError> {
    Instant::now()
        .checked_add(duration)
        .ok_or(PairingError::TimeOverflow)
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    let digest = digest(&SHA256, bytes);
    let mut output = [0_u8; 32];
    output.copy_from_slice(digest.as_ref());
    output
}

fn state_lock(state: &Mutex<ManagerState>) -> MutexGuard<'_, ManagerState> {
    state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn cell_lock<T>(state: &Mutex<T>) -> MutexGuard<'_, T> {
    state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
