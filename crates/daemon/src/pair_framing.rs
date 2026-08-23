//! Bounded framing for one short-lived pairing handshake.
//!
//! This module reuses [`zterm_proto::FrameDecoder`] for the wire format and
//! owns the single byte budget shared by both directions of a handshake. It
//! deliberately knows nothing about Iroh, pairing transcripts, or validated
//! domain conversion.

use std::collections::VecDeque;
use std::fmt;
use std::time::Instant;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use zeroize::{Zeroize, Zeroizing};
use zterm_core::{
    DomainErrorKind, MAX_PAIR_HELLO_FRAME_BYTES, PairHandshakeBudget, PairHandshakeBudgetError,
};
use zterm_proto::{DecodedFrame, FrameDecoder, ProtocolError, WireKind, encode_message};

use crate::error::DaemonError;

const READ_BUFFER_BYTES: usize = 4 * 1024;

/// Persistent decoder and shared traffic budget for one pairing handshake.
///
/// One value must be retained across all inbound and outbound frames so bytes
/// already read after a coalesced frame remain queued and both directions draw
/// from the same cumulative budget.
pub struct PairFraming {
    decoder: Option<FrameDecoder>,
    queued: VecDeque<DecodedFrame>,
    budget: PairHandshakeBudget,
    total_deadline: Instant,
    reached_eof: bool,
}

impl PairFraming {
    /// Creates framing with injected product limits and one absolute deadline.
    ///
    /// The per-frame body ceiling must be non-zero and no greater than the
    /// product's 16 KiB pairing-frame bound. The handshake byte ceiling is
    /// validated by [`PairHandshakeBudget`].
    pub fn new(
        maximum_frame_body_bytes: usize,
        maximum_handshake_bytes: usize,
        total_deadline: Instant,
    ) -> Result<Self, DaemonError> {
        if maximum_frame_body_bytes == 0 || maximum_frame_body_bytes > MAX_PAIR_HELLO_FRAME_BYTES {
            return Err(DaemonError::new(
                DomainErrorKind::ResourceExhausted,
                "pairing frame body ceiling must be between 1 byte and 16 KiB",
            ));
        }
        let budget = PairHandshakeBudget::with_maximum(maximum_handshake_bytes)
            .map_err(handshake_budget_error)?;
        Ok(Self {
            decoder: Some(FrameDecoder::with_maximum_body_bytes(
                maximum_frame_body_bytes,
            )),
            queued: VecDeque::new(),
            budget,
            total_deadline,
            reached_eof: false,
        })
    }

    /// Returns raw inbound plus fully framed outbound bytes already accounted.
    #[must_use]
    pub const fn used_bytes(&self) -> usize {
        self.budget.used()
    }

    /// Returns bytes left in the shared handshake budget.
    #[must_use]
    pub const fn remaining_bytes(&self) -> usize {
        self.budget.remaining()
    }

    /// Reads and decodes exactly the next expected pair-protocol message.
    ///
    /// `frame_deadline` is capped by the handshake's total deadline. Callers
    /// pass the stricter first-frame deadline for `PairBegin`, then the total
    /// deadline for later frames. Pair frames must use zero request/deadline
    /// metadata; domain conversion remains the caller's responsibility.
    pub async fn read_message<Reader, Message>(
        &mut self,
        reader: &mut Reader,
        expected: WireKind,
        frame_deadline: Instant,
    ) -> Result<Message, DaemonError>
    where
        Reader: AsyncRead + Unpin,
        Message: prost::Message + Default,
    {
        validate_pair_kind(expected)?;
        let deadline = self.effective_deadline(frame_deadline)?;
        let mut frame = self.next_frame(reader, deadline).await?;
        let result = validate_metadata(&frame)
            .and_then(|()| frame.decode_message(expected).map_err(protocol_error));
        frame.payload.zeroize();
        result
    }

    /// Encodes and writes one pair-protocol message with zero request metadata.
    ///
    /// The fully framed byte count is charged before the first byte is written.
    /// The encoded allocation is zeroized on success, timeout, or I/O failure.
    pub async fn write_message<Writer, Message>(
        &mut self,
        writer: &mut Writer,
        kind: WireKind,
        message: &Message,
        deadline: Instant,
    ) -> Result<(), DaemonError>
    where
        Writer: AsyncWrite + Unpin,
        Message: prost::Message,
    {
        validate_pair_kind(kind)?;
        let deadline = self.effective_deadline(deadline)?;
        let bytes = Zeroizing::new(encode_message(kind, 0, 0, message).map_err(protocol_error)?);
        self.budget
            .record(bytes.len())
            .map_err(handshake_budget_error)?;
        timeout_io(deadline, writer.write_all(bytes.as_slice()), "write").await
    }

    /// Gracefully shuts down a pairing writer within the absolute deadline.
    pub async fn shutdown<Writer>(
        &self,
        writer: &mut Writer,
        deadline: Instant,
    ) -> Result<(), DaemonError>
    where
        Writer: AsyncWrite + Unpin,
    {
        let deadline = self.effective_deadline(deadline)?;
        timeout_io(deadline, writer.shutdown(), "shutdown").await
    }

    async fn next_frame<Reader>(
        &mut self,
        reader: &mut Reader,
        deadline: Instant,
    ) -> Result<DecodedFrame, DaemonError>
    where
        Reader: AsyncRead + Unpin,
    {
        if let Some(frame) = self.queued.pop_front() {
            return Ok(frame);
        }
        if self.reached_eof {
            return Err(unexpected_eof());
        }

        let mut buffer = Zeroizing::new([0_u8; READ_BUFFER_BYTES]);
        loop {
            let allowed = self.budget.remaining().min(buffer.len());
            if allowed == 0 {
                return Err(DaemonError::new(
                    DomainErrorKind::ResourceExhausted,
                    "pairing handshake byte budget exhausted before the expected frame",
                ));
            }
            let read = timeout_io(deadline, reader.read(&mut buffer[..allowed]), "read").await?;
            if read == 0 {
                self.reached_eof = true;
                let decoder = self.decoder.take().ok_or_else(unexpected_eof)?;
                decoder.finish().map_err(protocol_error)?;
                return Err(unexpected_eof());
            }

            self.budget.record(read).map_err(handshake_budget_error)?;
            let decoded = self
                .decoder
                .as_mut()
                .ok_or_else(unexpected_eof)?
                .feed(&buffer[..read])
                .map_err(protocol_error)?;
            self.queued.extend(decoded);
            if let Some(frame) = self.queued.pop_front() {
                return Ok(frame);
            }
        }
    }

    fn effective_deadline(&self, operation_deadline: Instant) -> Result<Instant, DaemonError> {
        let deadline = self.total_deadline.min(operation_deadline);
        if Instant::now() >= deadline {
            return Err(deadline_error());
        }
        Ok(deadline)
    }
}

impl fmt::Debug for PairFraming {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PairFraming")
            .field("used_bytes", &self.budget.used())
            .field("maximum_bytes", &self.budget.maximum())
            .field("queued_frames", &self.queued.len())
            .field("reached_eof", &self.reached_eof)
            .finish_non_exhaustive()
    }
}

impl Drop for PairFraming {
    fn drop(&mut self) {
        for frame in &mut self.queued {
            frame.payload.zeroize();
        }
    }
}

fn validate_metadata(frame: &DecodedFrame) -> Result<(), DaemonError> {
    if frame.request_id != 0 || frame.deadline_ms != 0 {
        return Err(DaemonError::new(
            DomainErrorKind::MalformedFrame,
            "pairing frames must use zero request and deadline metadata",
        ));
    }
    Ok(())
}

fn validate_pair_kind(kind: WireKind) -> Result<(), DaemonError> {
    if matches!(
        kind,
        WireKind::PairBegin
            | WireKind::PairChallenge
            | WireKind::PairProof
            | WireKind::PairAccepted
    ) {
        return Ok(());
    }
    Err(DaemonError::new(
        DomainErrorKind::MalformedFrame,
        "message kind is not permitted on the pairing protocol",
    ))
}

fn protocol_error(error: ProtocolError) -> DaemonError {
    let kind = match error {
        ProtocolError::WireMajorMismatch { .. } => DomainErrorKind::WireMajorMismatch,
        ProtocolError::UnknownKind(_) => DomainErrorKind::UnknownKind,
        ProtocolError::FrameTooLarge(_) => DomainErrorKind::FrameTooLarge,
        ProtocolError::ControlPayloadTooLarge(_) => DomainErrorKind::ControlPayloadTooLarge,
        ProtocolError::MalformedVarint
        | ProtocolError::TruncatedFrame
        | ProtocolError::MalformedProtobuf(_)
        | ProtocolError::UnexpectedKind { .. }
        | ProtocolError::InvalidIdentifier(_)
        | ProtocolError::InvalidTerminalSize { .. } => DomainErrorKind::MalformedFrame,
    };
    DaemonError::new(kind, error.to_string())
}

fn handshake_budget_error(error: PairHandshakeBudgetError) -> DaemonError {
    DaemonError::new(DomainErrorKind::ResourceExhausted, error.to_string())
}

fn deadline_error() -> DaemonError {
    DaemonError::new(
        DomainErrorKind::DeadlineExceeded,
        "pairing frame deadline elapsed",
    )
}

fn unexpected_eof() -> DaemonError {
    DaemonError::new(
        DomainErrorKind::MalformedFrame,
        "pairing stream ended before the expected frame",
    )
}

async fn timeout_io<Output, Future>(
    deadline: Instant,
    future: Future,
    operation: &'static str,
) -> Result<Output, DaemonError>
where
    Future: std::future::Future<Output = std::io::Result<Output>>,
{
    if Instant::now() >= deadline {
        return Err(deadline_error());
    }
    tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), future)
        .await
        .map_err(|_| deadline_error())?
        .map_err(|_| {
            DaemonError::new(
                DomainErrorKind::TransportUnavailable,
                format!("pairing stream {operation} failed"),
            )
        })
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use zterm_core::{DomainErrorKind, MAX_PAIR_HANDSHAKE_BYTES, MAX_PAIR_HELLO_FRAME_BYTES};
    use zterm_proto::{WireKind, encode_message, v1};

    use super::PairFraming;

    const TEST_TIMEOUT: Duration = Duration::from_secs(2);
    const SECRET_SENTINEL: &[u8] = b"PAIR-FRAMING-SECRET-SENTINEL";

    fn deadline() -> Instant {
        Instant::now() + TEST_TIMEOUT
    }

    fn framing(total_deadline: Instant) -> PairFraming {
        PairFraming::new(
            MAX_PAIR_HELLO_FRAME_BYTES,
            MAX_PAIR_HANDSHAKE_BYTES,
            total_deadline,
        )
        .expect("production pairing framing limits are valid")
    }

    fn begin(name: &str) -> v1::PairBegin {
        v1::PairBegin {
            offer_id: vec![7; 16],
            controller_name: name.to_owned(),
            controller_nonce: vec![9; 32],
            pair_protocol_version: 1,
        }
    }

    fn proof(bytes: &[u8]) -> v1::PairProof {
        v1::PairProof {
            controller_proof: bytes.to_vec(),
        }
    }

    fn encoded<Message: prost::Message>(kind: WireKind, message: &Message) -> Vec<u8> {
        encode_message(kind, 0, 0, message).expect("test pair message fits the frame bound")
    }

    #[tokio::test]
    async fn partial_frame_is_retained_until_complete() {
        let total_deadline = deadline();
        let bytes = encoded(WireKind::PairBegin, &begin("controller"));
        let split = bytes.len() / 2;
        let expected_bytes = bytes.len();
        let (mut sender, mut receiver) = tokio::io::duplex(expected_bytes * 2);
        let write = async move {
            sender
                .write_all(&bytes[..split])
                .await
                .expect("first frame fragment writes");
            tokio::task::yield_now().await;
            sender
                .write_all(&bytes[split..])
                .await
                .expect("second frame fragment writes");
        };
        let read = async {
            let mut framing = framing(total_deadline);
            let decoded: v1::PairBegin = framing
                .read_message(&mut receiver, WireKind::PairBegin, total_deadline)
                .await
                .expect("partial frame decodes after completion");
            assert_eq!(decoded, begin("controller"));
            assert_eq!(framing.used_bytes(), expected_bytes);
        };

        tokio::join!(write, read);
    }

    #[tokio::test]
    async fn coalesced_frames_remain_queued_across_typed_reads() {
        let total_deadline = deadline();
        let begin = begin("controller");
        let proof = proof(&[4; 32]);
        let mut bytes = encoded(WireKind::PairBegin, &begin);
        bytes.extend_from_slice(&encoded(WireKind::PairProof, &proof));
        let expected_bytes = bytes.len();
        let mut input = bytes.as_slice();
        let mut framing = framing(total_deadline);

        let decoded_begin: v1::PairBegin = framing
            .read_message(&mut input, WireKind::PairBegin, total_deadline)
            .await
            .expect("first coalesced frame decodes");
        let decoded_proof: v1::PairProof = framing
            .read_message(&mut input, WireKind::PairProof, total_deadline)
            .await
            .expect("queued second frame decodes");

        assert_eq!(decoded_begin, begin);
        assert_eq!(decoded_proof, proof);
        assert_eq!(framing.used_bytes(), expected_bytes);
    }

    #[tokio::test]
    async fn queued_unexpected_kind_is_a_protocol_error() {
        let total_deadline = deadline();
        let begin = begin("controller");
        let mut bytes = encoded(WireKind::PairBegin, &begin);
        bytes.extend_from_slice(&encoded(WireKind::PairBegin, &begin));
        let mut input = bytes.as_slice();
        let mut framing = framing(total_deadline);

        let _: v1::PairBegin = framing
            .read_message(&mut input, WireKind::PairBegin, total_deadline)
            .await
            .expect("first frame has the expected kind");
        let error = framing
            .read_message::<_, v1::PairProof>(&mut input, WireKind::PairProof, total_deadline)
            .await
            .expect_err("queued extra kind must be rejected");

        assert_eq!(error.kind(), DomainErrorKind::MalformedFrame);
    }

    #[tokio::test]
    async fn nonzero_pair_metadata_is_rejected() {
        for (request_id, deadline_ms) in [(1, 0), (0, 1)] {
            let total_deadline = deadline();
            let bytes = encode_message(
                WireKind::PairBegin,
                request_id,
                deadline_ms,
                &begin("controller"),
            )
            .expect("test pair message encodes");
            let mut input = bytes.as_slice();
            let error = framing(total_deadline)
                .read_message::<_, v1::PairBegin>(&mut input, WireKind::PairBegin, total_deadline)
                .await
                .expect_err("pairing metadata must remain zero");
            assert_eq!(error.kind(), DomainErrorKind::MalformedFrame);
        }
    }

    #[tokio::test]
    async fn normal_protocol_kinds_are_rejected_before_io() {
        let total_deadline = deadline();
        let (_sender, mut receiver) = tokio::io::duplex(64);
        let mut framing = framing(total_deadline);
        let error = framing
            .read_message::<_, v1::ConnectionHello>(
                &mut receiver,
                WireKind::ConnectionHello,
                total_deadline,
            )
            .await
            .expect_err("normal protocol kind cannot enter pair framing");

        assert_eq!(error.kind(), DomainErrorKind::MalformedFrame);
        assert_eq!(framing.used_bytes(), 0);

        let (mut writer, mut observer) = tokio::io::duplex(64);
        let error = framing
            .write_message(
                &mut writer,
                WireKind::ConnectionHello,
                &v1::ConnectionHello::default(),
                total_deadline,
            )
            .await
            .expect_err("normal protocol kind cannot be written by pair framing");
        assert_eq!(error.kind(), DomainErrorKind::MalformedFrame);
        assert_eq!(framing.used_bytes(), 0);
        drop(writer);
        let mut observed = Vec::new();
        observer
            .read_to_end(&mut observed)
            .await
            .expect("observer reaches EOF without a normal frame");
        assert!(observed.is_empty());
    }

    #[tokio::test]
    async fn stricter_first_frame_deadline_preempts_total_deadline() {
        let total_deadline = Instant::now() + Duration::from_secs(60);
        let (_sender, mut receiver) = tokio::io::duplex(64);
        let error = framing(total_deadline)
            .read_message::<_, v1::PairBegin>(&mut receiver, WireKind::PairBegin, Instant::now())
            .await
            .expect_err("elapsed first-frame deadline must not await input");

        assert_eq!(error.kind(), DomainErrorKind::DeadlineExceeded);
    }

    #[tokio::test]
    async fn total_deadline_caps_a_later_operation_deadline() {
        let total_deadline = Instant::now();
        let (mut writer, mut observer) = tokio::io::duplex(64);
        let mut framing = framing(total_deadline);
        let error = framing
            .write_message(
                &mut writer,
                WireKind::PairProof,
                &proof(&[2; 32]),
                Instant::now() + Duration::from_secs(60),
            )
            .await
            .expect_err("elapsed total deadline must cap a later frame deadline");

        assert_eq!(error.kind(), DomainErrorKind::DeadlineExceeded);
        assert_eq!(framing.used_bytes(), 0);
        drop(writer);
        let mut observed = Vec::new();
        observer
            .read_to_end(&mut observed)
            .await
            .expect("observer reaches EOF without a late frame");
        assert!(observed.is_empty());
    }

    #[tokio::test]
    async fn partial_eof_is_a_typed_truncation_error() {
        let total_deadline = deadline();
        let mut bytes = encoded(WireKind::PairBegin, &begin("controller"));
        let _last = bytes.pop().expect("encoded frame is nonempty");
        let expected_bytes = bytes.len();
        let mut input = bytes.as_slice();
        let mut framing = framing(total_deadline);

        let error = framing
            .read_message::<_, v1::PairBegin>(&mut input, WireKind::PairBegin, total_deadline)
            .await
            .expect_err("partial EOF must be rejected as truncation");

        assert_eq!(error.kind(), DomainErrorKind::MalformedFrame);
        assert_eq!(framing.used_bytes(), expected_bytes);
    }

    #[tokio::test]
    async fn clean_eof_before_expected_frame_is_a_protocol_error() {
        let total_deadline = deadline();
        let mut input = &[][..];
        let error = framing(total_deadline)
            .read_message::<_, v1::PairBegin>(&mut input, WireKind::PairBegin, total_deadline)
            .await
            .expect_err("clean EOF cannot satisfy an expected pair frame");

        assert_eq!(error.kind(), DomainErrorKind::MalformedFrame);
    }

    #[tokio::test]
    async fn configured_frame_body_bound_rejects_before_message_decode() {
        let total_deadline = deadline();
        let bytes = encoded(WireKind::PairBegin, &begin(&"x".repeat(128)));
        let mut input = bytes.as_slice();
        let mut framing = PairFraming::new(32, MAX_PAIR_HANDSHAKE_BYTES, total_deadline)
            .expect("small nonzero injected frame limit is valid");

        let error = framing
            .read_message::<_, v1::PairBegin>(&mut input, WireKind::PairBegin, total_deadline)
            .await
            .expect_err("oversized pair body must be rejected");

        assert_eq!(error.kind(), DomainErrorKind::FrameTooLarge);
    }

    #[tokio::test]
    async fn inbound_and_outbound_bytes_share_one_budget() {
        let total_deadline = deadline();
        let inbound_message = begin("controller");
        let outbound_message = v1::PairChallenge {
            host_nonce: vec![3; 32],
            selected_version: 1,
            ticket_expiry_unix: 42,
        };
        let inbound = encoded(WireKind::PairBegin, &inbound_message);
        let outbound = encoded(WireKind::PairChallenge, &outbound_message);
        let maximum = inbound.len() + outbound.len() - 1;
        let mut input = inbound.as_slice();
        let mut framing = PairFraming::new(MAX_PAIR_HELLO_FRAME_BYTES, maximum, total_deadline)
            .expect("injected cumulative budget is valid");
        let _: v1::PairBegin = framing
            .read_message(&mut input, WireKind::PairBegin, total_deadline)
            .await
            .expect("inbound frame fits by itself");
        let inbound_used = framing.used_bytes();
        let (mut writer, mut observer) = tokio::io::duplex(outbound.len() * 2);

        let error = framing
            .write_message(
                &mut writer,
                WireKind::PairChallenge,
                &outbound_message,
                total_deadline,
            )
            .await
            .expect_err("fully framed outbound bytes exceed the shared budget");
        assert_eq!(error.kind(), DomainErrorKind::ResourceExhausted);
        assert_eq!(framing.used_bytes(), inbound_used);

        drop(writer);
        let mut observed = Vec::new();
        observer
            .read_to_end(&mut observed)
            .await
            .expect("duplex observer reaches EOF");
        assert!(observed.is_empty(), "budget rejection precedes every write");
    }

    #[tokio::test]
    async fn encoded_write_is_exact_and_shutdown_is_bounded() {
        let total_deadline = deadline();
        let message = v1::PairChallenge {
            host_nonce: vec![3; 32],
            selected_version: 1,
            ticket_expiry_unix: 42,
        };
        let expected = encoded(WireKind::PairChallenge, &message);
        let mut framing = framing(total_deadline);
        let (mut writer, mut observer) = tokio::io::duplex(expected.len() * 2);

        framing
            .write_message(
                &mut writer,
                WireKind::PairChallenge,
                &message,
                total_deadline,
            )
            .await
            .expect("bounded pair frame writes");
        framing
            .shutdown(&mut writer, total_deadline)
            .await
            .expect("writer shuts down before total deadline");
        let mut observed = Vec::new();
        observer
            .read_to_end(&mut observed)
            .await
            .expect("observer reads framed bytes through EOF");

        assert_eq!(observed, expected);
        assert_eq!(framing.used_bytes(), expected.len());
    }

    #[tokio::test]
    async fn queued_secret_payload_is_redacted_from_debug() {
        let total_deadline = deadline();
        let begin = begin("controller");
        let mut bytes = encoded(WireKind::PairBegin, &begin);
        bytes.extend_from_slice(&encoded(WireKind::PairProof, &proof(SECRET_SENTINEL)));
        let mut input = bytes.as_slice();
        let mut framing = framing(total_deadline);
        let _: v1::PairBegin = framing
            .read_message(&mut input, WireKind::PairBegin, total_deadline)
            .await
            .expect("first coalesced frame decodes");

        let debug = format!("{framing:?}");
        assert!(!debug.contains("PAIR-FRAMING-SECRET-SENTINEL"));
        assert!(!debug.contains(&format!("{:?}", SECRET_SENTINEL)));
    }

    #[test]
    fn invalid_injected_limits_are_rejected() {
        for frame_limit in [0, MAX_PAIR_HELLO_FRAME_BYTES + 1] {
            let error = PairFraming::new(frame_limit, MAX_PAIR_HANDSHAKE_BYTES, deadline())
                .expect_err("invalid pair frame limit must fail");
            assert_eq!(error.kind(), DomainErrorKind::ResourceExhausted);
        }
        let error = PairFraming::new(MAX_PAIR_HELLO_FRAME_BYTES, 0, deadline())
            .expect_err("zero handshake budget must fail");
        assert_eq!(error.kind(), DomainErrorKind::ResourceExhausted);
    }
}
