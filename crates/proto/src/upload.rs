//! Typed structural validation for the separate, non-replayed upload service.

use crate::{DecodedFrame, ProtocolError, WireKind, encode_message, v2};
use std::fmt;
use zterm_core::{
    DomainErrorKind,
    upload::{
        MAX_UPLOAD_BYTES, TransferId, UPLOAD_CHUNK_BYTES, UploadBinding, UploadMetadata,
        UploadedFile,
    },
};

impl fmt::Debug for v2::UploadChunk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UploadChunk")
            .field("offset", &self.offset)
            .field("data_len", &self.data.len())
            .finish_non_exhaustive()
    }
}
impl fmt::Debug for v2::UploadCompleted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UploadCompleted")
            .field("size", &self.size)
            .field("path", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

/// Validated frames. Stream owners enforce order, correlation and cumulative bytes.
#[allow(missing_docs)]
pub enum UploadMessage {
    Begin {
        binding: UploadBinding,
        metadata: UploadMetadata,
    },
    Ready {
        id: TransferId,
        size: u64,
    },
    Chunk {
        id: TransferId,
        offset: u64,
        data: Vec<u8>,
    },
    Progress {
        id: TransferId,
        accepted_bytes: u64,
    },
    Finish {
        id: TransferId,
        size: u64,
    },
    Completed {
        id: TransferId,
        file: UploadedFile,
    },
    Cancel {
        id: TransferId,
    },
    Cancelled {
        id: TransferId,
    },
}

impl fmt::Debug for UploadMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UploadMessage")
            .field("kind", &self.kind())
            .finish_non_exhaustive()
    }
}

impl UploadMessage {
    /// Stable wire registry kind.
    #[must_use]
    pub const fn kind(&self) -> WireKind {
        match self {
            Self::Begin { .. } => WireKind::UploadBegin,
            Self::Ready { .. } => WireKind::UploadReady,
            Self::Chunk { .. } => WireKind::UploadChunk,
            Self::Progress { .. } => WireKind::UploadProgress,
            Self::Finish { .. } => WireKind::UploadFinish,
            Self::Completed { .. } => WireKind::UploadCompleted,
            Self::Cancel { .. } => WireKind::UploadCancel,
            Self::Cancelled { .. } => WireKind::UploadCancelled,
        }
    }

    /// Encodes a correlated frame using the unchanged global framing limits.
    pub fn encode(&self, request_id: u64) -> Result<Vec<u8>, ProtocolError> {
        if request_id == 0 {
            return Err(invalid());
        }
        let kind = self.kind();
        match self {
            Self::Begin { binding, metadata } => encode_message(
                kind,
                request_id,
                0,
                &v2::UploadBegin {
                    session_id: Some(binding.session_id.into()),
                    attachment_id: Some(binding.attachment_id.into()),
                    size: metadata.size(),
                    extension: metadata.extension().into(),
                },
            ),
            Self::Ready { id, size } => encode_message(
                kind,
                request_id,
                0,
                &v2::UploadReady {
                    transfer_id: id.as_bytes().to_vec(),
                    size: *size,
                },
            ),
            Self::Chunk { id, offset, data } => encode_message(
                kind,
                request_id,
                0,
                &v2::UploadChunk {
                    transfer_id: id.as_bytes().to_vec(),
                    offset: *offset,
                    data: data.clone(),
                },
            ),
            Self::Progress { id, accepted_bytes } => encode_message(
                kind,
                request_id,
                0,
                &v2::UploadProgress {
                    transfer_id: id.as_bytes().to_vec(),
                    accepted_bytes: *accepted_bytes,
                },
            ),
            Self::Finish { id, size } => encode_message(
                kind,
                request_id,
                0,
                &v2::UploadFinish {
                    transfer_id: id.as_bytes().to_vec(),
                    size: *size,
                },
            ),
            Self::Completed { id, file } => encode_message(
                kind,
                request_id,
                0,
                &v2::UploadCompleted {
                    transfer_id: id.as_bytes().to_vec(),
                    size: file.size(),
                    path: file.path().into(),
                },
            ),
            Self::Cancel { id } => encode_message(
                kind,
                request_id,
                0,
                &v2::UploadCancel {
                    transfer_id: id.as_bytes().to_vec(),
                },
            ),
            Self::Cancelled { id } => encode_message(
                kind,
                request_id,
                0,
                &v2::UploadCancelled {
                    transfer_id: id.as_bytes().to_vec(),
                },
            ),
        }
    }

    /// Decodes required IDs, limits and path text at the wire boundary.
    pub fn decode(frame: &DecodedFrame) -> Result<Self, ProtocolError> {
        if frame.request_id == 0 {
            return Err(invalid());
        }
        Ok(match frame.kind {
            WireKind::UploadBegin => {
                let message: v2::UploadBegin = frame.decode_message(frame.kind)?;
                let metadata = UploadMetadata::new(message.size, &message.extension)
                    .map_err(ProtocolError::InvalidUpload)?;
                if metadata.extension() != message.extension {
                    return Err(invalid());
                }
                Self::Begin {
                    binding: UploadBinding {
                        session_id: message.session_id.ok_or_else(invalid)?.try_into()?,
                        attachment_id: message.attachment_id.ok_or_else(invalid)?.try_into()?,
                    },
                    metadata,
                }
            }
            WireKind::UploadReady => {
                let message: v2::UploadReady = frame.decode_message(frame.kind)?;
                Self::Ready {
                    id: id(&message.transfer_id)?,
                    size: size(message.size)?,
                }
            }
            WireKind::UploadChunk => {
                let message: v2::UploadChunk = frame.decode_message(frame.kind)?;
                if message.data.is_empty()
                    || message.data.len() > UPLOAD_CHUNK_BYTES
                    || message
                        .offset
                        .checked_add(message.data.len() as u64)
                        .is_none_or(|end| end > MAX_UPLOAD_BYTES)
                {
                    return Err(invalid());
                }
                Self::Chunk {
                    id: id(&message.transfer_id)?,
                    offset: message.offset,
                    data: message.data,
                }
            }
            WireKind::UploadProgress => {
                let message: v2::UploadProgress = frame.decode_message(frame.kind)?;
                Self::Progress {
                    id: id(&message.transfer_id)?,
                    accepted_bytes: size(message.accepted_bytes)?,
                }
            }
            WireKind::UploadFinish => {
                let message: v2::UploadFinish = frame.decode_message(frame.kind)?;
                Self::Finish {
                    id: id(&message.transfer_id)?,
                    size: size(message.size)?,
                }
            }
            WireKind::UploadCompleted => {
                let message: v2::UploadCompleted = frame.decode_message(frame.kind)?;
                Self::Completed {
                    id: id(&message.transfer_id)?,
                    file: UploadedFile::new(message.path, message.size)
                        .map_err(ProtocolError::InvalidUpload)?,
                }
            }
            WireKind::UploadCancel => {
                let message: v2::UploadCancel = frame.decode_message(frame.kind)?;
                Self::Cancel {
                    id: id(&message.transfer_id)?,
                }
            }
            WireKind::UploadCancelled => {
                let message: v2::UploadCancelled = frame.decode_message(frame.kind)?;
                Self::Cancelled {
                    id: id(&message.transfer_id)?,
                }
            }
            _ => return Err(invalid()),
        })
    }
}

fn invalid() -> ProtocolError {
    ProtocolError::InvalidUpload(DomainErrorKind::MalformedFrame)
}
fn id(bytes: &[u8]) -> Result<TransferId, ProtocolError> {
    TransferId::from_bytes(bytes).map_err(ProtocolError::InvalidIdentifier)
}
fn size(value: u64) -> Result<u64, ProtocolError> {
    if value > MAX_UPLOAD_BYTES {
        Err(ProtocolError::InvalidUpload(
            DomainErrorKind::UploadTooLarge,
        ))
    } else {
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FrameDecoder;
    use zterm_core::{AttachmentId, SessionId};

    fn frame(bytes: &[u8]) -> DecodedFrame {
        FrameDecoder::new()
            .feed(bytes)
            .expect("upload fixture")
            .remove(0)
    }

    #[test]
    fn upload_registry_round_trips_and_preserves_frame_limits() {
        let id = TransferId::from_array([7; 16]);
        let messages = [
            UploadMessage::Begin {
                binding: UploadBinding {
                    session_id: SessionId::from_array([1; 16]),
                    attachment_id: AttachmentId::from_array([2; 16]),
                },
                metadata: UploadMetadata::new(MAX_UPLOAD_BYTES, "pdf").expect("upload fixture"),
            },
            UploadMessage::Ready { id, size: 0 },
            UploadMessage::Chunk {
                id,
                offset: 0,
                data: vec![0x83; UPLOAD_CHUNK_BYTES],
            },
            UploadMessage::Progress {
                id,
                accepted_bytes: MAX_UPLOAD_BYTES,
            },
            UploadMessage::Finish { id, size: 0 },
            UploadMessage::Completed {
                id,
                file: UploadedFile::new("/tmp/zterm-1/session/transfer/file.pdf".into(), 0)
                    .expect("upload fixture"),
            },
            UploadMessage::Cancel { id },
            UploadMessage::Cancelled { id },
        ];
        for (offset, message) in messages.iter().enumerate() {
            let kind = 400 + offset as u32;
            assert_eq!(
                WireKind::try_from(kind).expect("upload fixture"),
                message.kind()
            );
            assert!(v2::MessageKind::try_from(kind as i32).is_ok());
            let encoded = message.encode(9).expect("upload fixture");
            assert_eq!(
                UploadMessage::decode(&frame(&encoded))
                    .expect("upload fixture")
                    .encode(9)
                    .expect("upload fixture"),
                encoded
            );
        }
        assert_eq!(crate::MAX_FRAME_BYTES, 8 * 1024 * 1024);
        assert_eq!(crate::MAX_CONTROL_PAYLOAD_BYTES, 1024 * 1024);
    }

    #[test]
    fn malformed_ids_offsets_and_oversized_data_fail_before_service_use() {
        for message in [
            v2::UploadChunk {
                transfer_id: vec![1; 15],
                offset: 0,
                data: vec![1],
            },
            v2::UploadChunk {
                transfer_id: vec![1; 16],
                offset: MAX_UPLOAD_BYTES,
                data: vec![1],
            },
            v2::UploadChunk {
                transfer_id: vec![1; 16],
                offset: 0,
                data: vec![1; UPLOAD_CHUNK_BYTES + 1],
            },
            v2::UploadChunk {
                transfer_id: vec![1; 16],
                offset: 0,
                data: Vec::new(),
            },
        ] {
            let bytes =
                encode_message(WireKind::UploadChunk, 1, 0, &message).expect("upload fixture");
            assert!(UploadMessage::decode(&frame(&bytes)).is_err());
        }
        let missing = v2::UploadBegin::default();
        assert!(
            UploadMessage::decode(&frame(
                &encode_message(WireKind::UploadBegin, 1, 0, &missing).expect("upload fixture")
            ))
            .is_err()
        );
        let result = v2::UploadCompleted {
            transfer_id: vec![1; 16],
            size: 1,
            path: "/tmp/zterm-1/file\ncommand".into(),
        };
        assert!(
            UploadMessage::decode(&frame(
                &encode_message(WireKind::UploadCompleted, 1, 0, &result).expect("upload fixture")
            ))
            .is_err()
        );
    }
}
