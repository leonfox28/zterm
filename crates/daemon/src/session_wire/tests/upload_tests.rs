use super::*;
use tokio_util::sync::CancellationToken;
use zterm_client::{
    transport::TransportFuture,
    upload::{self, AsyncUploadWriter, FramedUploadReader, UploadConnection, UploadConnector},
};
use zterm_core::{Capabilities, upload::*};

struct Fixture {
    server: SessionWireServer,
    context: SessionRequestContext,
    prepared: PreparedAttachment,
    tasks: Mutex<Vec<tokio::task::JoinHandle<Result<(), DaemonError>>>>,
    _tmp: tempfile::TempDir,
}
impl Fixture {
    fn new() -> Self {
        let own = device(0xd1);
        let remote = device(0xd2);
        let accepted = generation(1);
        let tmp = tempfile::tempdir().expect("upload fixture");
        let sessions = unix_wire_service(own, tmp.path().to_path_buf());
        let context = remote_context(own, remote, accepted, authorized_registry(remote, accepted));
        let prepared = sessions
            .prepare_attach(
                context.principal(&sessions),
                None,
                true,
                false,
                Some(TerminalSize::new(24, 80)),
            )
            .expect("upload fixture");
        activate_attachment(&prepared);
        Self {
            server: SessionWireServer::new(sessions),
            context,
            prepared,
            tasks: Mutex::new(Vec::new()),
            _tmp: tmp,
        }
    }
    fn binding(&self) -> UploadBinding {
        UploadBinding {
            session_id: self.prepared.attachment.session_id(),
            attachment_id: self.prepared.attachment.attachment_id(),
        }
    }
    async fn join(&self) {
        let tasks = std::mem::take(&mut *self.tasks.lock().expect("upload fixture"));
        for task in tasks {
            let _ = tokio::time::timeout(Duration::from_secs(5), task)
                .await
                .expect("upload fixture")
                .expect("upload fixture");
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.server.sessions.shutdown();
    }
}
impl UploadConnector for Fixture {
    fn open_upload(
        &self,
        _target: zterm_client::model::ResolvedSessionTarget,
    ) -> TransportFuture<'_, Result<UploadConnection, DaemonError>> {
        Box::pin(async move {
            // Smaller than a chunk: full-duplex flow control is necessary to finish.
            let (peer, stream) = tokio::io::duplex(4096);
            let server = self.server.clone();
            let context = self.context.clone();
            self.tasks
                .lock()
                .expect("upload fixture")
                .push(tokio::spawn(async move {
                    server
                        .handle_remote_stream(
                            stream,
                            context,
                            SessionWireLimits::default(),
                            Instant::now() + Duration::from_secs(5),
                        )
                        .await
                }));
            let (reader, writer) = tokio::io::split(peer);
            Ok(UploadConnection {
                capabilities: Capabilities::from_bits_retain(Capabilities::FILE_UPLOAD_SERVICE),
                reader: Box::new(FramedUploadReader::new(reader, ())),
                writer: Box::new(AsyncUploadWriter(writer)),
            })
        })
    }
}

fn target() -> zterm_client::model::ResolvedSessionTarget {
    zterm_client::model::ResolvedSessionTarget::device(device(0xd1))
}
fn remove_published(file: &UploadedFile) {
    let path = Path::new(file.path());
    std::fs::remove_file(path).expect("upload fixture");
    std::fs::remove_dir(path.parent().expect("upload fixture")).expect("upload fixture");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn upload_round_trip_accepts_zero_and_exact_limit_over_bounded_duplex() {
    let fixture = Fixture::new();
    let (progress, _) = tokio::sync::watch::channel(upload::preparing());
    for length in [0, 2_000_013, MAX_UPLOAD_BYTES as usize] {
        let bytes: Vec<u8> = (0..length).map(|index| (index % 251) as u8).collect();
        let file = upload::upload(
            &fixture,
            target(),
            fixture.binding(),
            UploadMetadata::new(length as u64, "pdf").expect("upload fixture"),
            bytes.as_slice(),
            &progress,
            &CancellationToken::new(),
        )
        .await
        .expect("upload fixture");
        assert_eq!(std::fs::read(file.path()).expect("upload fixture"), bytes);
        assert_eq!(progress.borrow().phase, UploadPhase::Completed);
        assert_eq!(progress.borrow().accepted_bytes, length as u64);
        assert!(
            fixture
                .server
                .sessions
                .admit_upload_until(
                    fixture.context.principal(&fixture.server.sessions),
                    fixture.binding(),
                    Instant::now() + Duration::from_secs(2)
                )
                .is_ok(),
            "upload admission must never detach the original controller"
        );
        remove_published(&file);
    }
    fixture.join().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn source_changes_and_wrong_attachment_cannot_publish() {
    let fixture = Fixture::new();
    let (progress, _) = tokio::sync::watch::channel(upload::preparing());
    for (size, bytes) in [(2, b"abc".as_slice()), (4, b"abc".as_slice())] {
        let error = upload::upload(
            &fixture,
            target(),
            fixture.binding(),
            UploadMetadata::new(size, "").expect("upload fixture"),
            bytes,
            &progress,
            &CancellationToken::new(),
        )
        .await
        .expect_err("upload must fail");
        assert_eq!(error.kind(), DomainErrorKind::UploadSourceInvalid);
    }
    let wrong = UploadBinding {
        attachment_id: AttachmentId::from_array([0; 16]),
        ..fixture.binding()
    };
    let error = upload::upload(
        &fixture,
        target(),
        wrong,
        UploadMetadata::new(0, "").expect("upload fixture"),
        [].as_slice(),
        &progress,
        &CancellationToken::new(),
    )
    .await
    .expect_err("upload must fail");
    assert_eq!(error.kind(), DomainErrorKind::LeaseLost);
    fixture.join().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_and_detach_retire_staging_while_terminal_survives() {
    let fixture = Fixture::new();
    let (progress, mut observed) = tokio::sync::watch::channel(upload::preparing());
    for detach in [false, true] {
        progress.send_replace(upload::preparing());
        observed.borrow_and_update();
        let cancellation = CancellationToken::new();
        let (mut writer, source) = tokio::io::duplex(4096);
        let feeding = async {
            let _ = writer.write_all(&vec![3; UPLOAD_CHUNK_BYTES]).await;
            std::future::pending::<()>().await;
        };
        let action = async {
            loop {
                observed.changed().await.expect("upload fixture");
                if observed.borrow().accepted_bytes > 0 {
                    break;
                }
            }
            if detach {
                fixture.prepared.attachment.detach();
            } else {
                cancellation.cancel();
            }
        };
        let operation = upload::upload(
            &fixture,
            target(),
            fixture.binding(),
            UploadMetadata::new(1_000_000, "png").expect("upload fixture"),
            source,
            &progress,
            &cancellation,
        );
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::select! { result = operation => result, () = async { tokio::join!(feeding, action); } => unreachable!() }
        }).await.expect("upload fixture").expect_err("upload must fail");
        assert_eq!(
            result.kind(),
            if detach {
                DomainErrorKind::LeaseLost
            } else {
                DomainErrorKind::Cancelled
            }
        );
        fixture.join().await;
    }
    assert_eq!(
        fixture
            .server
            .sessions
            .list()
            .expect("upload fixture")
            .len(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_streams_remove_staging_without_disturbing_controller() {
    use zterm_proto::upload::UploadMessage;
    let fixture = Fixture::new();
    for case in 0..5 {
        let mut connection = fixture
            .open_upload(target())
            .await
            .expect("fixture connection");
        connection
            .writer
            .write(
                &UploadMessage::Begin {
                    binding: fixture.binding(),
                    metadata: UploadMetadata::new(3, "pdf").expect("metadata"),
                }
                .encode(1)
                .expect("encode begin"),
            )
            .await
            .expect("send begin");
        let UploadMessage::Ready { id, .. } =
            UploadMessage::decode(&connection.reader.read().await.expect("ready"))
                .expect("decode ready")
        else {
            panic!("ready expected")
        };
        let transfer: String = id
            .as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let path = format!(
            "/tmp/zterm-{}/{}/{transfer}",
            nix::unistd::geteuid(),
            fixture.binding().session_id
        );
        assert!(Path::new(&path).join(".part").is_file());
        let message = match case {
            0 => UploadMessage::Chunk {
                id: TransferId::from_array([0; 16]),
                offset: 0,
                data: vec![1],
            },
            1 => UploadMessage::Chunk {
                id,
                offset: 1,
                data: vec![1],
            },
            2 => UploadMessage::Finish { id, size: 3 },
            3 => UploadMessage::Begin {
                binding: fixture.binding(),
                metadata: UploadMetadata::new(0, "").expect("metadata"),
            },
            _ => UploadMessage::Chunk {
                id,
                offset: 0,
                data: vec![1],
            },
        };
        connection
            .writer
            .write(
                &message
                    .encode(if case == 4 { 2 } else { 1 })
                    .expect("encode invalid sequence"),
            )
            .await
            .expect("send invalid sequence");
        let response = connection.reader.read().await.expect("structured error");
        assert_eq!(response.kind, WireKind::ServiceErrorResponse);
        fixture.join().await;
        assert!(
            !Path::new(&path).exists(),
            "failed stream must remove only its staging"
        );
        assert!(
            fixture
                .server
                .sessions
                .admit_upload_until(
                    fixture.context.principal(&fixture.server.sessions),
                    fixture.binding(),
                    Instant::now() + Duration::from_secs(2)
                )
                .is_ok()
        );
    }
}
