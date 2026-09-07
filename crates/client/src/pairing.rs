//! Shared controller pairing and authenticated transcript rules.

use crate::{error::ClientError, pair_framing::PairFraming};
use std::{fmt, time::Instant};
use tokio::io::{AsyncRead, AsyncWrite};
use zeroize::{Zeroize, Zeroizing};
use zterm_core::{
    AuthGeneration, DeviceDisplayName, DeviceId, DomainErrorKind, PAIR_PROTOCOL_VERSION, PairBegin,
    PairChallenge, PairNonce, PairSecret, PairTicketError, PairTicketFields, PairTranscript,
    RelayHint, TransportLimits, TransportLimitsError,
};
use zterm_proto::{WireKind, v2};

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
    pub fn peer_error(self) -> ClientError {
        let detail = match self.peer_kind() {
            DomainErrorKind::ResourceExhausted => "pairing service is overloaded",
            DomainErrorKind::TransportUnavailable => "pairing service is unavailable",
            DomainErrorKind::DeadlineExceeded => "pairing handshake deadline elapsed",
            _ => "pairing request was rejected",
        };
        ClientError::new(self.peer_kind(), detail)
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

impl From<PairingError> for ClientError {
    fn from(error: PairingError) -> Self {
        Self::new(error.local_kind(), error.to_string())
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

/// Authenticated duplex stream for one transient pair handshake.
pub trait PairProtocolIo: AsyncRead + AsyncWrite + Unpin + Send {
    /// Locally authenticated endpoint identity.
    fn local(&self) -> DeviceId;
    /// TLS-authenticated peer identity.
    fn remote(&self) -> DeviceId;
}

/// Authorization established on the normal application ALPN.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfirmedAuthorization {
    /// TLS peer which sent the validated Welcome.
    pub remote: DeviceId,
    /// Nonzero authorization generation from Welcome.
    pub generation: AuthGeneration,
    /// Route verified during normal authentication.
    pub verified_relay: Option<RelayHint>,
}

/// Pair-ALPN outcome awaiting authoritative normal confirmation.
pub struct ControllerPairAttempt {
    accepted_generation: Option<AuthGeneration>,
    repair_required: bool,
    error: Option<ClientError>,
}

impl ControllerPairAttempt {
    /// Records failure, including whether a proof may have committed remotely.
    pub fn failed(error: ClientError, repair_required: bool) -> Self {
        Self {
            accepted_generation: None,
            repair_required,
            error: Some(error),
        }
    }
}

/// Resolves lost or ambiguous pair replies using the normal ALPN authority.
pub fn resolve_normal_confirmation(
    expected_remote: DeviceId,
    pair_attempt: ControllerPairAttempt,
    confirmation: Result<ConfirmedAuthorization, ClientError>,
) -> Result<ConfirmedAuthorization, ClientError> {
    let confirmation = match confirmation {
        Ok(confirmation)
            if confirmation.remote == expected_remote
                && confirmation.generation != AuthGeneration::ZERO =>
        {
            confirmation
        }
        Ok(_) => {
            return Err(pair_outcome_unknown(
                "normal confirmation returned inconsistent authorization",
            ));
        }
        Err(normal_error) => {
            if pair_attempt.repair_required {
                return Err(pair_outcome_unknown(
                    "remote pairing outcome could not be confirmed",
                ));
            }
            return Err(pair_attempt.error.unwrap_or(normal_error));
        }
    };
    if pair_attempt
        .accepted_generation
        .is_some_and(|accepted| confirmation.generation < accepted)
    {
        return Err(pair_outcome_unknown(
            "normal confirmation generation predates PairAccepted",
        ));
    }
    Ok(confirmation)
}

/// Durable local identity bound into the controller transcript.
pub struct ControllerPairIdentity<'a> {
    /// Locally authenticated endpoint public key.
    pub device_id: DeviceId,
    /// Validated local diagnostic name.
    pub display_name: &'a str,
}

/// Executes the pair exchange on an authenticated stream, retaining ambiguity.
/// Call normal confirmation even after failure and persist before reporting success.
pub async fn run_controller_pair(
    mut io: Box<dyn PairProtocolIo>,
    fields: &PairTicketFields,
    secret: &PairSecret,
    identity: ControllerPairIdentity<'_>,
    limits: &TransportLimits,
    deadline: Instant,
    nonce: impl FnOnce() -> Result<PairNonce, ClientError>,
) -> ControllerPairAttempt {
    if io.local() != identity.device_id
        || io.remote() != fields.host_device_id()
        || io.local() == io.remote()
    {
        return ControllerPairAttempt::failed(
            invalid_pair_ticket("pairing TLS identity did not match the ticket"),
            false,
        );
    }

    let mut framing = match PairFraming::new(
        limits.max_pair_hello_frame_bytes,
        limits.max_pair_handshake_bytes,
        deadline,
    ) {
        Ok(framing) => framing,
        Err(error) => return ControllerPairAttempt::failed(error, false),
    };
    let controller_nonce = match nonce() {
        Ok(nonce) => nonce,
        Err(error) => return ControllerPairAttempt::failed(error, false),
    };
    let begin = match PairBegin::new(
        fields.offer_id(),
        identity.display_name,
        controller_nonce,
        PAIR_PROTOCOL_VERSION,
    ) {
        Ok(begin) => begin,
        Err(_) => {
            return ControllerPairAttempt::failed(
                invalid_pair_ticket("local pairing identity is invalid"),
                false,
            );
        }
    };
    let mut begin_wire = v2::PairBegin::from(&begin);
    let begin_write = framing
        .write_message(&mut io, WireKind::PairBegin, &begin_wire, deadline)
        .await;
    begin_wire.offer_id.zeroize();
    begin_wire.controller_nonce.zeroize();
    if let Err(error) = begin_write {
        return ControllerPairAttempt::failed(error, false);
    }

    let challenge_wire: v2::PairChallenge = match framing
        .read_message(&mut io, WireKind::PairChallenge, deadline)
        .await
    {
        Ok(challenge) => challenge,
        Err(error) => return ControllerPairAttempt::failed(error, false),
    };
    let challenge = match pair_challenge_from_wire(challenge_wire) {
        Ok(challenge) => challenge,
        Err(error) => return ControllerPairAttempt::failed(error, false),
    };
    let transcript =
        match controller_transcript(fields, io.remote(), identity.device_id, &begin, &challenge) {
            Ok(transcript) => transcript,
            Err(error) => return ControllerPairAttempt::failed(error.into(), false),
        };
    let offer_key = Zeroizing::new(fields.offer_key(secret));
    let controller_proof = Zeroizing::new(transcript.controller_proof(&offer_key));
    let mut proof_wire = v2::PairProof {
        controller_proof: controller_proof.to_vec(),
    };
    // A cancelled write_all may already have delivered a complete frame;
    // normal confirmation is therefore mandatory from this point onward.
    let proof_write = framing
        .write_message(&mut io, WireKind::PairProof, &proof_wire, deadline)
        .await;
    proof_wire.controller_proof.zeroize();
    if let Err(error) = proof_write {
        return ControllerPairAttempt::failed(error, true);
    }

    let mut accepted_wire: v2::PairAccepted = match framing
        .read_message(&mut io, WireKind::PairAccepted, deadline)
        .await
    {
        Ok(accepted) => accepted,
        Err(error) => return ControllerPairAttempt::failed(error, true),
    };
    let accepted = validate_pair_accepted(&mut accepted_wire, &transcript, &offer_key);
    let generation = match accepted {
        Ok(generation) => generation,
        Err(error) => return ControllerPairAttempt::failed(error, true),
    };
    if let Err(error) = framing.shutdown(&mut io, deadline).await {
        return ControllerPairAttempt {
            accepted_generation: Some(generation),
            repair_required: true,
            error: Some(error),
        };
    }
    ControllerPairAttempt {
        accepted_generation: Some(generation),
        repair_required: true,
        error: None,
    }
}

fn invalid_pair_ticket(detail: &'static str) -> ClientError {
    ClientError::new(DomainErrorKind::PairTicketInvalid, detail)
}

fn pair_outcome_unknown(detail: &'static str) -> ClientError {
    ClientError::new(DomainErrorKind::PairOutcomeUnknown, detail)
}

fn pair_challenge_from_wire(mut wire: v2::PairChallenge) -> Result<PairChallenge, ClientError> {
    let nonce = PairNonce::from_bytes(&wire.host_nonce)
        .map_err(|_| invalid_pair_ticket("pairing challenge nonce is invalid"));
    wire.host_nonce.zeroize();
    PairChallenge::new(nonce?, wire.selected_version, wire.ticket_expiry_unix)
        .map_err(|_| invalid_pair_ticket("pairing challenge fields are invalid"))
}

fn validate_pair_accepted(
    wire: &mut v2::PairAccepted,
    transcript: &zterm_core::PairTranscript,
    offer_key: &[u8; 32],
) -> Result<AuthGeneration, ClientError> {
    let generation = AuthGeneration::new(wire.authorization_generation)
        .filter(|generation| *generation != AuthGeneration::ZERO)
        .ok_or_else(|| invalid_pair_ticket("PairAccepted generation is invalid"));
    let proof = <[u8; 32]>::try_from(wire.host_confirmation_proof.as_slice())
        .map(Zeroizing::new)
        .map_err(|_| invalid_pair_ticket("PairAccepted confirmation proof is invalid"));
    wire.host_confirmation_proof.zeroize();
    DeviceDisplayName::new(std::mem::take(&mut wire.host_diagnostic_version))
        .map_err(|_| invalid_pair_ticket("PairAccepted diagnostic version is invalid"))?;
    let generation = generation?;
    let proof = proof?;
    if !transcript.verify_host_confirmation(offer_key, generation.get(), &proof) {
        return Err(invalid_pair_ticket(
            "PairAccepted confirmation proof was rejected",
        ));
    }
    Ok(generation)
}
