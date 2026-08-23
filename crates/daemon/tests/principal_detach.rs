//! Remote-principal detach and takeover ownership integration tests.

#[cfg(unix)]
#[path = "support/session_fixture.rs"]
mod support;

#[cfg(unix)]
use zterm_core::{AttachmentPrincipal, DeviceId};

#[cfg(unix)]
#[test]
fn remote_detach_releases_only_matching_attachments_across_sessions() -> Result<(), String> {
    use zterm_core::{DomainErrorKind, ResourceLimits, SessionSelector};

    let fixture = support::Fixture::new(ResourceLimits::default())?;
    let a = fixture.create(1, "a").map_err(support::display)?;
    let b = fixture.create(2, "b").map_err(support::display)?;
    let c = fixture.create(3, "c").map_err(support::display)?;

    // A same-UID local principal controls `c`.
    let local = fixture
        .service
        .prepare_attach(
            fixture.principal,
            Some(SessionSelector::Id(c.session_id)),
            false,
            false,
            None,
        )
        .map_err(support::display)?;
    support::activate(&local)?;

    // Two distinct remote principals control `a` and `b`.
    let (r1_id, r1) = remote_principal(0x11);
    let (_r2_id, r2) = remote_principal(0x22);
    let a_attach = fixture
        .service
        .prepare_attach(
            r1,
            Some(SessionSelector::Id(a.session_id)),
            false,
            false,
            None,
        )
        .map_err(support::display)?;
    support::activate(&a_attach)?;
    let b_attach = fixture
        .service
        .prepare_attach(
            r2,
            Some(SessionSelector::Id(b.session_id)),
            false,
            false,
            None,
        )
        .map_err(support::display)?;
    support::activate(&b_attach)?;

    let impact = fixture
        .service
        .detach_remote_principal(r1_id)
        .map_err(support::display)?;
    assert_eq!(impact.sessions_affected, 1);
    assert_eq!(impact.attachments_removed, 1);
    assert_eq!(impact.controllers_released, 1);

    // The detached remote attachment is stale for every effect.
    assert_eq!(
        a_attach
            .attachment
            .write_input(b"must-not-write")
            .expect_err("detached remote attachment cannot write")
            .kind(),
        DomainErrorKind::LeaseLost
    );

    // The other remote and the local principal keep their controllers.
    b_attach
        .attachment
        .write_input(b"printf 'R2-STILL-CONTROLLER\\n'\n")
        .map_err(support::display)?;
    support::wait_for_text(&b_attach.attachment, "R2-STILL-CONTROLLER")?;
    local
        .attachment
        .write_input(b"printf 'LOCAL-STILL-CONTROLLER\\n'\n")
        .map_err(support::display)?;
    support::wait_for_text(&local.attachment, "LOCAL-STILL-CONTROLLER")?;

    // No session or PTY was closed by detaching the principal.
    assert_eq!(fixture.service.list().map_err(support::display)?.len(), 3);
    fixture.service.shutdown().map_err(support::display)?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn remote_detach_is_idempotent_and_ignores_gone_sessions() -> Result<(), String> {
    use zterm_core::{ResourceLimits, SessionSelector};

    let fixture = support::Fixture::new(ResourceLimits::default())?;
    let a = fixture.create(1, "a").map_err(support::display)?;
    let (r1_id, r1) = remote_principal(0x33);
    let a_attach = fixture
        .service
        .prepare_attach(
            r1,
            Some(SessionSelector::Id(a.session_id)),
            false,
            false,
            None,
        )
        .map_err(support::display)?;
    support::activate(&a_attach)?;

    let first = fixture
        .service
        .detach_remote_principal(r1_id)
        .map_err(support::display)?;
    assert_eq!(first.attachments_removed, 1);

    // Repeating the same detach is a no-op.
    let second = fixture
        .service
        .detach_remote_principal(r1_id)
        .map_err(support::display)?;
    assert_eq!(second.sessions_affected, 0);
    assert_eq!(second.attachments_removed, 0);
    assert_eq!(second.controllers_released, 0);

    // A principal that never attached also reports a clean zero impact.
    let (_never_id, _never) = remote_principal(0x44);
    let none = fixture
        .service
        .detach_remote_principal(_never_id)
        .map_err(support::display)?;
    assert_eq!(none.attachments_removed, 0);

    // Detaching after the session has already ended (the natural-exit/detach
    // race) must not error even though the actor is gone from the registry.
    let d = fixture.create(2, "d").map_err(support::display)?;
    let (r2_id, r2) = remote_principal(0x55);
    let d_attach = fixture
        .service
        .prepare_attach(
            r2,
            Some(SessionSelector::Id(d.session_id)),
            false,
            false,
            None,
        )
        .map_err(support::display)?;
    support::activate(&d_attach)?;
    fixture
        .service
        .close(fixture.principal, fixture.op(3), d.session_id)
        .map_err(support::display)?;
    support::wait_for_session_count(&fixture.service, 1)?;
    let after_end = fixture
        .service
        .detach_remote_principal(r2_id)
        .map_err(support::display)?;
    assert_eq!(after_end.attachments_removed, 0);

    fixture.service.shutdown().map_err(support::display)?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn takeover_requires_the_preparing_principal_and_stale_effects_are_rejected() -> Result<(), String>
{
    use zterm_core::{DomainErrorKind, OperationId, ResourceLimits, SessionSelector};

    let fixture = support::Fixture::new(ResourceLimits::default())?;
    let a = fixture.create(1, "a").map_err(support::display)?;
    let (r1_id, r1) = remote_principal(0x66);
    let a_attach = fixture
        .service
        .prepare_attach(
            r1,
            Some(SessionSelector::Id(a.session_id)),
            false,
            false,
            None,
        )
        .map_err(support::display)?;
    support::activate(&a_attach)?;

    // A different principal cannot take over an attachment prepared by `r1`.
    let (_r2_id, r2) = remote_principal(0x77);
    let r2_lease = fixture
        .service
        .issue_operation_lease(r2)
        .map_err(support::display)?;
    let mismatch = fixture
        .service
        .takeover(
            r2,
            OperationId {
                lease: r2_lease,
                sequence: 1,
            },
            &a_attach.attachment,
        )
        .expect_err("a different principal cannot take over this attachment");
    assert_eq!(mismatch.kind(), DomainErrorKind::LeaseLost);

    // After detaching `r1`, input, resize, and takeover are all rejected.
    fixture
        .service
        .detach_remote_principal(r1_id)
        .map_err(support::display)?;
    assert_eq!(
        a_attach
            .attachment
            .write_input(b"stale")
            .expect_err("stale input")
            .kind(),
        DomainErrorKind::LeaseLost
    );
    assert_eq!(
        a_attach
            .attachment
            .resize(support::default_size())
            .expect_err("stale resize")
            .kind(),
        DomainErrorKind::LeaseLost
    );
    let r1_lease = fixture
        .service
        .issue_operation_lease(r1)
        .map_err(support::display)?;
    let stale_takeover = fixture
        .service
        .takeover(
            r1,
            OperationId {
                lease: r1_lease,
                sequence: 1,
            },
            &a_attach.attachment,
        )
        .expect_err("stale takeover");
    assert_eq!(stale_takeover.kind(), DomainErrorKind::LeaseLost);

    fixture.service.shutdown().map_err(support::display)?;
    Ok(())
}

#[cfg(unix)]
fn remote_principal(byte: u8) -> (DeviceId, AttachmentPrincipal) {
    let device_id = DeviceId::from_array([byte; 32]);
    let principal = AttachmentPrincipal::RemoteEndpoint {
        device_id,
        auth_generation: 1,
    };
    (device_id, principal)
}
