use super::*;
use zterm_core::terminal::{COLOR_BACKGROUND, TerminalAppearance, TerminalColorValue};

fn profile(value: u8) -> TerminalColorProfile {
    let mut p = TerminalColorProfile {
        appearance: TerminalAppearance::Light,
        ..Default::default()
    };
    p.values[COLOR_BACKGROUND] = TerminalColorValue::Rgb(value, value, value);
    p
}
fn acknowledge(prepared: &PreparedAttachment) {
    let mut revision = prepared.snapshot.revision;
    while let Some(snapshot) = prepared
        .attachment
        .snapshot_applied(revision)
        .expect("color fixture operation succeeds")
    {
        revision = snapshot.revision;
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn controller_colors_are_staged_for_takeover_and_require_a_new_ack() {
    let temporary = tempfile::tempdir().expect("color fixture operation succeeds");
    let service = unix_fixture_service(
        DeviceId::from_array([0xc1; 32]),
        temporary.path().to_path_buf(),
        "stty -echo; printf 'READY\\r\\n'; while IFS= read -r line; do case \"$line\" in theme) printf '\\033]11;red\\007OVERRIDE\\r\\n';; reset) printf '\\033]111\\007RESET\\r\\n';; esac; done",
    );
    let principal = service.local_principal(AttachmentId::from_array([0xc1; 16]));
    let first = service
        .prepare_attach_with_colors_until(
            principal,
            None,
            true,
            false,
            crate::session::InitialTerminal {
                viewport: None,
                colors: profile(200),
            },
            default_deadline(),
        )
        .expect("color fixture operation succeeds");
    assert_eq!(first.snapshot.surface.colors.profile, profile(200));
    acknowledge(&first);
    wait_for_attachment_text(&first, b"READY").await;
    let id = first.attachment.session_id();
    let pending = service
        .prepare_attach_with_colors_until(
            principal,
            Some(SessionSelector::Id(id)),
            false,
            true,
            crate::session::InitialTerminal {
                viewport: None,
                colors: profile(30),
            },
            default_deadline(),
        )
        .expect("color fixture operation succeeds");
    assert_eq!(pending.snapshot.surface.colors.profile, profile(200));
    assert!(
        pending
            .attachment
            .update_colors_until(1, profile(10), default_deadline())
            .is_err()
    );
    acknowledge(&pending);
    let lease = service
        .issue_operation_lease(principal)
        .expect("color fixture operation succeeds");
    service
        .takeover(
            principal,
            OperationId { lease, sequence: 1 },
            &pending.attachment,
        )
        .expect("color fixture operation succeeds");
    assert_eq!(
        first
            .attachment
            .update_colors_until(1, profile(99), default_deadline())
            .expect_err("invalid color operation must fail")
            .kind(),
        DomainErrorKind::LeaseLost
    );
    assert_eq!(
        pending
            .attachment
            .write_input(b"theme\n")
            .expect_err("invalid color operation must fail")
            .kind(),
        DomainErrorKind::NotSynchronized
    );
    let Some(AttachmentUpdate::Snapshot(replacement)) = pending
        .attachment
        .next_update()
        .expect("color fixture operation succeeds")
    else {
        panic!("takeover replaces the old color snapshot");
    };
    assert_eq!(replacement.surface.colors.profile, profile(30));
    assert!(
        pending
            .attachment
            .snapshot_applied(replacement.revision)
            .expect("color fixture operation succeeds")
            .is_none()
    );
    pending
        .attachment
        .write_input(b"theme\n")
        .expect("color fixture operation succeeds");
    wait_for_attachment_text(&pending, b"OVERRIDE").await;
    pending
        .attachment
        .update_colors_until(1, profile(80), default_deadline())
        .expect("color fixture operation succeeds");
    assert!(
        pending
            .attachment
            .update_colors_until(1, profile(90), default_deadline())
            .is_err()
    );
    let overridden = pending
        .attachment
        .sync_latest(Revision::default())
        .expect("color fixture operation succeeds");
    assert_eq!(
        overridden.surface.colors.profile.values[COLOR_BACKGROUND],
        TerminalColorValue::Rgb(255, 0, 0)
    );
    pending
        .attachment
        .snapshot_applied(overridden.revision)
        .expect("color fixture operation succeeds");
    pending.attachment.detach();
    let next = service
        .prepare_attach_with_colors_until(
            principal,
            Some(SessionSelector::Id(id)),
            false,
            false,
            crate::session::InitialTerminal {
                viewport: None,
                colors: profile(120),
            },
            default_deadline(),
        )
        .expect("color fixture operation succeeds");
    assert_eq!(
        next.snapshot.surface.colors.profile.values[COLOR_BACKGROUND],
        TerminalColorValue::Rgb(255, 0, 0)
    );
    acknowledge(&next);
    next.attachment
        .write_input(b"reset\n")
        .expect("color fixture operation succeeds");
    wait_for_attachment_text(&next, b"RESET").await;
    let restored = next
        .attachment
        .sync_latest(Revision::default())
        .expect("color fixture operation succeeds");
    assert_eq!(
        restored.surface.colors.profile.values[COLOR_BACKGROUND],
        profile(120).values[COLOR_BACKGROUND]
    );
    service
        .shutdown()
        .expect("color fixture operation succeeds");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn named_create_supplies_colors_before_startup_query_and_fingerprints_them() {
    let temporary = tempfile::tempdir().expect("color fixture operation succeeds");
    // The child cannot create this file until its first OSC query was answered.
    // The test waits for it before opening any attachment.
    let service = unix_fixture_service(
        DeviceId::from_array([0xc2; 32]),
        temporary.path().to_path_buf(),
        "stty -echo -icanon min 1 time 0; printf '\\033]11;?\\007'; dd bs=1 count=24 of=answer 2>/dev/null; printf 'STARTED\\r\\n'; exec /bin/cat",
    );
    let principal = service.local_principal(AttachmentId::from_array([0xc2; 16]));
    let lease = service
        .issue_operation_lease(principal)
        .expect("color fixture operation succeeds");
    let operation = OperationId { lease, sequence: 1 };
    let summary = service
        .create_with_colors_until(
            principal,
            operation,
            SessionName::new("colored").expect("color fixture operation succeeds"),
            None,
            crate::session::InitialTerminal {
                viewport: None,
                colors: profile(80),
            },
            default_deadline(),
        )
        .expect("color fixture operation succeeds");
    let answer = temporary.path().join("answer");
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if std::fs::read(&answer).is_ok_and(|bytes| bytes.len() == 24) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("startup query answered before attach");
    assert_eq!(
        std::fs::read(&answer).expect("color fixture operation succeeds"),
        b"\x1b]11;rgb:5050/5050/5050\x07"
    );
    assert!(
        service
            .create_with_colors_until(
                principal,
                operation,
                SessionName::new("colored").expect("color fixture operation succeeds"),
                None,
                crate::session::InitialTerminal {
                    viewport: None,
                    colors: profile(81)
                },
                default_deadline()
            )
            .is_err()
    );
    let first = service
        .prepare_attach_with_colors_until(
            principal,
            Some(SessionSelector::Id(summary.session_id)),
            false,
            false,
            crate::session::InitialTerminal {
                viewport: None,
                colors: profile(80),
            },
            default_deadline(),
        )
        .expect("color fixture operation succeeds");
    assert_eq!(first.snapshot.surface.colors.profile, profile(80));
    service
        .shutdown()
        .expect("color fixture operation succeeds");
}
