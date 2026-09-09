//! Upload stream owner: existing authorization/controller admission plus off-actor disk IO.

use super::*;
use zterm_client::framing::FrameReader;
use zterm_core::upload::*;
use zterm_platform::upload::StagedUpload;
use zterm_proto::upload::UploadMessage;

impl SessionWireServer {
    pub(super) async fn handle_upload<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>(
        &self,
        stream: S,
        first: FirstFrame,
        context: SessionRequestContext,
    ) -> Result<(), DaemonError> {
        let request_id = first.frame.request_id;
        let begin = UploadMessage::decode(&first.frame).map_err(protocol_error);
        let (reader, mut writer) = tokio::io::split(stream);
        let mut reader = FrameReader::after_first(reader, first);
        let result = self
            .upload_stream(&mut reader, &mut writer, begin, request_id, &context)
            .await;
        if let Err(error) = &result {
            write_error_best_effort(
                &mut writer,
                request_id,
                error,
                Instant::now() + DEFAULT_SESSION_WIRE_DEADLINE,
            )
            .await;
        }
        result
    }

    async fn upload_stream<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
        &self,
        reader: &mut FrameReader<R>,
        writer: &mut W,
        begin: Result<UploadMessage, DaemonError>,
        request_id: u64,
        context: &SessionRequestContext,
    ) -> Result<(), DaemonError> {
        let UploadMessage::Begin { binding, metadata } = begin? else {
            return Err(malformed("upload stream must start with Begin"));
        };
        let mut admission = context
            .run_effect(
                &self.sessions,
                control_deadline(),
                move |sessions, principal| {
                    sessions.admit_upload_until(principal, binding, control_deadline())
                },
            )
            .await?;
        let random = iroh::SecretKey::generate().to_bytes();
        let id = TransferId::from_bytes(&random[..TransferId::LENGTH])
            .expect("fixed random transfer ID");
        let size = metadata.size();
        let mut staged = run_blocking_until(control_deadline(), move || {
            StagedUpload::create(binding.session_id, id, metadata).map_err(storage_error)
        })
        .await?;
        if !admission.is_current() {
            return Err(upload_lease_lost());
        }
        write_upload(writer, request_id, UploadMessage::Ready { id, size }).await?;
        let mut acknowledged = 0;
        let mut idle = tokio::time::Instant::now() + UPLOAD_IDLE_TIMEOUT;
        let mut tick = tokio::time::interval(UPLOAD_PROGRESS_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            if !admission.is_current() {
                return Err(upload_lease_lost());
            }
            let frame = tokio::select! {
                biased;
                changed = admission.lifecycle.changed() => {
                    if changed.is_err() || !admission.is_current() { return Err(upload_lease_lost()); }
                    continue;
                }
                _ = tokio::time::sleep_until(idle) => return Err(DaemonError::new(DomainErrorKind::DeadlineExceeded, "upload stream became idle")),
                _ = tick.tick(), if staged.accepted_bytes() > acknowledged => {
                    acknowledged = staged.accepted_bytes();
                    write_upload(writer, request_id, UploadMessage::Progress { id, accepted_bytes: acknowledged }).await?;
                    continue;
                }
                frame = reader.read() => frame?,
            };
            if frame.request_id != request_id || frame.deadline_ms != 0 {
                return Err(malformed("upload request correlation mismatch"));
            }
            match UploadMessage::decode(&frame).map_err(protocol_error)? {
                UploadMessage::Chunk {
                    id: received,
                    offset,
                    data,
                } if received == id => {
                    if offset != staged.accepted_bytes() || offset + data.len() as u64 > size {
                        return Err(malformed("upload chunk offset or size mismatch"));
                    }
                    staged = run_blocking_until(Instant::now() + UPLOAD_IDLE_TIMEOUT, move || {
                        staged.write_chunk(offset, &data).map_err(storage_error)?;
                        Ok(staged)
                    })
                    .await?;
                    idle = tokio::time::Instant::now() + UPLOAD_IDLE_TIMEOUT;
                    if staged.accepted_bytes() - acknowledged >= UPLOAD_ACK_BYTES
                        || staged.accepted_bytes() == size
                    {
                        acknowledged = staged.accepted_bytes();
                        write_upload(
                            writer,
                            request_id,
                            UploadMessage::Progress {
                                id,
                                accepted_bytes: acknowledged,
                            },
                        )
                        .await?;
                    }
                }
                UploadMessage::Cancel { id: received } if received == id => {
                    drop(staged);
                    return write_upload(writer, request_id, UploadMessage::Cancelled { id }).await;
                }
                UploadMessage::Finish {
                    id: received,
                    size: finished,
                } if received == id && finished == size && staged.accepted_bytes() == size => {
                    let generation = admission.generation;
                    let file = context
                        .run_effect(
                            &self.sessions,
                            control_deadline(),
                            move |sessions, principal| {
                                let current = sessions.admit_upload_until(
                                    principal,
                                    binding,
                                    control_deadline(),
                                )?;
                                if current.generation != generation || !current.is_current() {
                                    return Err(upload_lease_lost());
                                }
                                staged.publish().map_err(|_| DaemonError::new(
                                    DomainErrorKind::UploadOutcomeUnknown,
                                    "host could not confirm publication; a remote file may exist",
                                ))
                            },
                        )
                        .await?;
                    // If control changed during disk publication, keep the file but
                    // do not return a pasteable result to the retired attachment.
                    if !admission.is_current() {
                        return Err(upload_lease_lost());
                    }
                    return write_upload(writer, request_id, UploadMessage::Completed { id, file })
                        .await;
                }
                _ => {
                    return Err(malformed(
                        "unexpected upload message, transfer ID or final byte count",
                    ));
                }
            }
        }
    }
}

fn control_deadline() -> Instant {
    Instant::now() + DEFAULT_SESSION_WIRE_DEADLINE
}
fn storage_error(_: std::io::Error) -> DaemonError {
    DaemonError::new(
        DomainErrorKind::UploadStorageFailed,
        "host could not write or publish the upload",
    )
}
fn upload_lease_lost() -> DaemonError {
    DaemonError::new(
        DomainErrorKind::LeaseLost,
        "upload attachment no longer controls the session",
    )
}
async fn write_upload<W: AsyncWrite + Unpin>(
    writer: &mut W,
    request_id: u64,
    message: UploadMessage,
) -> Result<(), DaemonError> {
    let bytes = message.encode(request_id).map_err(protocol_error)?;
    timeout_at(
        control_deadline(),
        writer.write_all(&bytes),
        "upload response write timed out",
    )
    .await?
    .map_err(|_| {
        DaemonError::new(
            DomainErrorKind::TransportUnavailable,
            "upload response stream closed",
        )
    })
}
