//! Single-controller synchronization and explicit takeover integration tests.

#[cfg(unix)]
#[path = "support/session_fixture.rs"]
mod support;

#[cfg(unix)]
#[test]
fn takeover_requires_snapshot_and_invalidates_the_old_controller() -> Result<(), String> {
    use zterm_core::{DomainErrorKind, ResourceLimits, SessionSelector};
    use zterm_daemon::session::AttachmentLifecycle;

    let fixture = support::Fixture::new(ResourceLimits::default())?;
    let first = fixture
        .service
        .prepare_attach(None, true, false, None)
        .map_err(support::display)?;
    support::activate(&first)?;
    let occupied = fixture.service.prepare_attach(
        Some(SessionSelector::Id(first.attachment.session_id())),
        false,
        false,
        None,
    );
    let occupied = match occupied {
        Ok(_) => return Err("ordinary second controller unexpectedly succeeded".into()),
        Err(error) => error,
    };
    assert_eq!(occupied.kind(), DomainErrorKind::SessionOccupied);

    let pending = fixture
        .service
        .prepare_attach(
            Some(SessionSelector::Id(first.attachment.session_id())),
            false,
            true,
            None,
        )
        .map_err(support::display)?;
    assert_eq!(
        pending
            .attachment
            .write_input(b"must-not-write")
            .expect_err("pending takeover cannot write")
            .kind(),
        DomainErrorKind::NotSynchronized
    );
    let second_pending = fixture.service.prepare_attach(
        Some(SessionSelector::Id(first.attachment.session_id())),
        false,
        true,
        None,
    );
    let second_pending = match second_pending {
        Ok(_) => return Err("a second pending takeover unexpectedly allocated state".into()),
        Err(error) => error,
    };
    assert_eq!(second_pending.kind(), DomainErrorKind::SessionOccupied);
    support::activate(&pending)?;
    let mut old_lifecycle = first
        .attachment
        .lifecycle_watch()
        .map_err(support::display)?;
    let summary = fixture
        .service
        .takeover(fixture.principal, fixture.op(1), &pending.attachment)
        .map_err(support::display)?;
    assert_eq!(summary.session_id, first.attachment.session_id());
    assert!(matches!(
        old_lifecycle.borrow_and_update().clone(),
        AttachmentLifecycle::LeaseLost { .. }
    ));
    assert_eq!(
        first
            .attachment
            .write_input(b"old-must-not-write")
            .expect_err("stale controller")
            .kind(),
        DomainErrorKind::LeaseLost
    );
    pending
        .attachment
        .write_input(b"printf 'NEW-CONTROLLER\\n'\n")
        .map_err(support::display)?;
    support::wait_for_text(&pending.attachment, "NEW-CONTROLLER")?;

    let replay = fixture
        .service
        .takeover(fixture.principal, fixture.op(1), &pending.attachment)
        .map_err(support::display)?;
    assert_eq!(replay, summary);

    let replacement = fixture
        .service
        .prepare_attach(
            Some(SessionSelector::Id(first.attachment.session_id())),
            false,
            true,
            None,
        )
        .map_err(support::display)?;
    support::activate(&replacement)?;
    let continued = fixture
        .service
        .takeover(fixture.principal, fixture.op(1), &replacement.attachment)
        .map_err(support::display)?;
    assert_eq!(continued, summary);
    assert_eq!(
        pending
            .attachment
            .write_input(b"old-replayed-controller")
            .expect_err("response-loss continuation detaches old controller")
            .kind(),
        DomainErrorKind::LeaseLost
    );
    replacement
        .attachment
        .write_input(b"printf 'RECONNECTED-CONTROLLER\\n'\n")
        .map_err(support::display)?;
    support::wait_for_text(&replacement.attachment, "RECONNECTED-CONTROLLER")?;
    fixture.service.shutdown().map_err(support::display)?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn same_operation_continuation_reacquires_authority_after_old_controller_detaches()
-> Result<(), String> {
    use zterm_core::{ResourceLimits, Revision, SessionSelector};

    let fixture = support::Fixture::new(ResourceLimits::default())?;
    let original = fixture
        .service
        .prepare_attach(None, true, false, None)
        .map_err(support::display)?;
    support::activate(&original)?;
    let session_id = original.attachment.session_id();
    let first = fixture
        .service
        .prepare_attach(Some(SessionSelector::Id(session_id)), false, true, None)
        .map_err(support::display)?;
    support::activate(&first)?;
    let retained = fixture
        .service
        .takeover(fixture.principal, fixture.op(10), &first.attachment)
        .map_err(support::display)?;
    first.attachment.detach();

    // Preparing the replacement is itself ordered through the actor and thus
    // deterministically reaps the detached controller before continuation.
    let replacement = fixture
        .service
        .prepare_attach(Some(SessionSelector::Id(session_id)), false, true, None)
        .map_err(support::display)?;
    support::activate(&replacement)?;
    let continued = fixture
        .service
        .takeover(fixture.principal, fixture.op(10), &replacement.attachment)
        .map_err(support::display)?;
    assert_eq!(continued, retained);
    replacement
        .attachment
        .write_input(b"printf 'DETACHED-RETRY-CONTROLLER\\n'\n")
        .map_err(support::display)?;
    support::wait_for_text(&replacement.attachment, "DETACHED-RETRY-CONTROLLER")?;
    assert!(replacement.attachment.sync_latest(Revision::ZERO).is_ok());
    fixture.service.shutdown().map_err(support::display)?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn stale_same_operation_continuation_cannot_clobber_a_later_controller() -> Result<(), String> {
    use zterm_core::{DomainErrorKind, ResourceLimits, SessionSelector};

    let fixture = support::Fixture::new(ResourceLimits::default())?;
    let original = fixture
        .service
        .prepare_attach(None, true, false, None)
        .map_err(support::display)?;
    support::activate(&original)?;
    let session_id = original.attachment.session_id();
    let old = fixture
        .service
        .prepare_attach(Some(SessionSelector::Id(session_id)), false, true, None)
        .map_err(support::display)?;
    support::activate(&old)?;
    fixture
        .service
        .takeover(fixture.principal, fixture.op(20), &old.attachment)
        .map_err(support::display)?;

    let later = fixture
        .service
        .prepare_attach(Some(SessionSelector::Id(session_id)), false, true, None)
        .map_err(support::display)?;
    support::activate(&later)?;
    fixture
        .service
        .takeover(fixture.principal, fixture.op(21), &later.attachment)
        .map_err(support::display)?;

    let stale = fixture
        .service
        .prepare_attach(Some(SessionSelector::Id(session_id)), false, true, None)
        .map_err(support::display)?;
    support::activate(&stale)?;
    let rejected = fixture
        .service
        .takeover(fixture.principal, fixture.op(20), &stale.attachment)
        .expect_err("old replay continuation cannot replace a later operation");
    assert_eq!(rejected.kind(), DomainErrorKind::OperationOutcomeUnknown);
    later
        .attachment
        .write_input(b"printf 'LATER-CONTROLLER-STILL-ACTIVE\\n'\n")
        .map_err(support::display)?;
    support::wait_for_text(&later.attachment, "LATER-CONTROLLER-STILL-ACTIVE")?;
    fixture.service.shutdown().map_err(support::display)?;
    Ok(())
}
