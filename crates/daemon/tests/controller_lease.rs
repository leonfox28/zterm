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
        .prepare_attach(fixture.principal, None, true, false, None)
        .map_err(support::display)?;
    support::activate(&first)?;
    let occupied = fixture.service.prepare_attach(
        fixture.principal,
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
            fixture.principal,
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
        fixture.principal,
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
            fixture.principal,
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
        .prepare_attach(fixture.principal, None, true, false, None)
        .map_err(support::display)?;
    support::activate(&original)?;
    let session_id = original.attachment.session_id();
    let first = fixture
        .service
        .prepare_attach(
            fixture.principal,
            Some(SessionSelector::Id(session_id)),
            false,
            true,
            None,
        )
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
        .prepare_attach(
            fixture.principal,
            Some(SessionSelector::Id(session_id)),
            false,
            true,
            None,
        )
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
        .prepare_attach(fixture.principal, None, true, false, None)
        .map_err(support::display)?;
    support::activate(&original)?;
    let session_id = original.attachment.session_id();
    let old = fixture
        .service
        .prepare_attach(
            fixture.principal,
            Some(SessionSelector::Id(session_id)),
            false,
            true,
            None,
        )
        .map_err(support::display)?;
    support::activate(&old)?;
    fixture
        .service
        .takeover(fixture.principal, fixture.op(20), &old.attachment)
        .map_err(support::display)?;

    let later = fixture
        .service
        .prepare_attach(
            fixture.principal,
            Some(SessionSelector::Id(session_id)),
            false,
            true,
            None,
        )
        .map_err(support::display)?;
    support::activate(&later)?;
    fixture
        .service
        .takeover(fixture.principal, fixture.op(21), &later.attachment)
        .map_err(support::display)?;

    let stale = fixture
        .service
        .prepare_attach(
            fixture.principal,
            Some(SessionSelector::Id(session_id)),
            false,
            true,
            None,
        )
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

#[cfg(unix)]
#[test]
fn remote_created_session_survives_bidirectional_cross_principal_takeover() -> Result<(), String> {
    use zterm_core::{
        AttachmentPrincipal, DeviceId, DomainErrorKind, OperationId, ResourceLimits, SessionName,
        SessionSelector,
    };
    use zterm_daemon::session::AttachmentLifecycle;

    let fixture = support::Fixture::new(ResourceLimits::default())?;
    let remote = AttachmentPrincipal::RemoteEndpoint {
        device_id: DeviceId::from_array([0x41; DeviceId::LENGTH]),
        auth_generation: 3,
    };
    let other_remote = AttachmentPrincipal::RemoteEndpoint {
        device_id: DeviceId::from_array([0x42; DeviceId::LENGTH]),
        auth_generation: 5,
    };
    let remote_lease = fixture
        .service
        .issue_operation_lease(remote)
        .map_err(support::display)?;
    let other_lease = fixture
        .service
        .issue_operation_lease(other_remote)
        .map_err(support::display)?;

    let created = fixture
        .service
        .create(
            remote,
            OperationId {
                lease: remote_lease,
                sequence: 1,
            },
            SessionName::new("remote-origin").expect("fixture Session name"),
            Some(fixture.temp.path().to_path_buf()),
            Some(support::default_size()),
        )
        .map_err(|_| "remote-origin Session creation failed".to_owned())?;
    if created.working_directory != fixture.temp.path() {
        return Err("remote-created Session did not retain its requested cwd identity".into());
    }
    let remote_controller = fixture
        .service
        .prepare_attach(
            remote,
            Some(SessionSelector::Id(created.session_id)),
            false,
            false,
            None,
        )
        .map_err(support::display)?;
    support::activate(&remote_controller)?;
    assert_eq!(
        remote_controller.attachment.session_id(),
        created.session_id
    );
    remote_controller
        .attachment
        .write_input(b"ZTERM_STEP8_PROCESS=origin; printf 'REMOTE-READY\\n'\n")
        .map_err(support::display)?;
    wait_for_safe_marker(&remote_controller.attachment, "REMOTE-READY")?;

    let other = fixture
        .service
        .create(
            other_remote,
            OperationId {
                lease: other_lease,
                sequence: 1,
            },
            SessionName::new("other-principal").expect("fixture Session name"),
            None,
            None,
        )
        .map_err(|_| "other-principal Session creation failed".to_owned())?;
    let other_controller = fixture
        .service
        .prepare_attach(
            other_remote,
            Some(SessionSelector::Id(other.session_id)),
            false,
            false,
            None,
        )
        .map_err(support::display)?;
    support::activate(&other_controller)?;
    other_controller
        .attachment
        .write_input(b"ZTERM_STEP8_OTHER=alive; printf 'OTHER-%s\\n' \"$ZTERM_STEP8_OTHER\"\n")
        .map_err(support::display)?;
    wait_for_safe_marker(&other_controller.attachment, "OTHER-alive")?;

    let occupied = fixture.service.prepare_attach(
        fixture.principal,
        Some(SessionSelector::Id(created.session_id)),
        false,
        false,
        None,
    );
    let occupied = match occupied {
        Ok(_) => return Err("ordinary host-local attach unexpectedly stole the controller".into()),
        Err(error) => error,
    };
    assert_eq!(occupied.kind(), DomainErrorKind::SessionOccupied);
    remote_controller
        .attachment
        .write_input(b"printf 'REMOTE-STILL-CONTROLS\\n'\n")
        .map_err(support::display)?;
    wait_for_safe_marker(&remote_controller.attachment, "REMOTE-STILL-CONTROLS")?;

    let local_takeover = fixture
        .service
        .prepare_attach(
            fixture.principal,
            Some(SessionSelector::Id(created.session_id)),
            false,
            true,
            None,
        )
        .map_err(support::display)?;
    assert_eq!(local_takeover.attachment.session_id(), created.session_id);
    if !safe_snapshot_contains(&local_takeover.snapshot, "REMOTE-STILL-CONTROLS")? {
        return Err("host-local takeover did not receive the existing authoritative screen".into());
    }
    support::activate(&local_takeover)?;
    let mut remote_lifecycle = remote_controller
        .attachment
        .lifecycle_watch()
        .map_err(support::display)?;
    let local_summary = fixture
        .service
        .takeover(fixture.principal, fixture.op(1), &local_takeover.attachment)
        .map_err(support::display)?;
    assert_eq!(local_summary.session_id, created.session_id);
    if local_summary.working_directory != created.working_directory {
        return Err("host-local takeover changed the Session cwd identity".into());
    }
    assert!(matches!(
        remote_lifecycle.borrow_and_update().clone(),
        AttachmentLifecycle::LeaseLost { .. }
    ));
    assert_eq!(
        remote_controller
            .attachment
            .write_input(b"stale-remote-input")
            .expect_err("replaced remote lease cannot write")
            .kind(),
        DomainErrorKind::LeaseLost
    );
    // The expected expanded value is not present in these input bytes, so
    // observing it proves the remote-created shell process retained state.
    local_takeover
        .attachment
        .write_input(b"printf 'LOCAL-%s\\n' \"$ZTERM_STEP8_PROCESS\"\n")
        .map_err(support::display)?;
    wait_for_safe_marker(&local_takeover.attachment, "LOCAL-origin")?;

    let reverse_occupied = fixture.service.prepare_attach(
        remote,
        Some(SessionSelector::Id(created.session_id)),
        false,
        false,
        None,
    );
    let reverse_occupied = match reverse_occupied {
        Ok(_) => return Err("ordinary remote attach unexpectedly stole the controller".into()),
        Err(error) => error,
    };
    assert_eq!(reverse_occupied.kind(), DomainErrorKind::SessionOccupied);
    local_takeover
        .attachment
        .write_input(b"printf 'LOCAL-STILL-CONTROLS\\n'\n")
        .map_err(support::display)?;
    wait_for_safe_marker(&local_takeover.attachment, "LOCAL-STILL-CONTROLS")?;

    let remote_return = fixture
        .service
        .prepare_attach(
            remote,
            Some(SessionSelector::Id(created.session_id)),
            false,
            true,
            None,
        )
        .map_err(support::display)?;
    assert_eq!(remote_return.attachment.session_id(), created.session_id);
    if !safe_snapshot_contains(&remote_return.snapshot, "LOCAL-STILL-CONTROLS")? {
        return Err("reverse takeover did not retain the host-local screen update".into());
    }
    support::activate(&remote_return)?;
    let mut local_lifecycle = local_takeover
        .attachment
        .lifecycle_watch()
        .map_err(support::display)?;
    let remote_summary = fixture
        .service
        .takeover(
            remote,
            OperationId {
                lease: remote_lease,
                sequence: 2,
            },
            &remote_return.attachment,
        )
        .map_err(support::display)?;
    assert_eq!(remote_summary.session_id, created.session_id);
    if remote_summary.working_directory != created.working_directory {
        return Err("reverse takeover changed the Session cwd identity".into());
    }
    assert!(matches!(
        local_lifecycle.borrow_and_update().clone(),
        AttachmentLifecycle::LeaseLost { .. }
    ));
    assert_eq!(
        local_takeover
            .attachment
            .write_input(b"stale-local-input")
            .expect_err("replaced local lease cannot write")
            .kind(),
        DomainErrorKind::LeaseLost
    );
    // The reverse expansion proves the same shell survived both atomic
    // controller transfers rather than only preserving a copied screen.
    remote_return
        .attachment
        .write_input(b"printf 'REMOTE-%s\\n' \"$ZTERM_STEP8_PROCESS\"\n")
        .map_err(support::display)?;
    wait_for_safe_marker(&remote_return.attachment, "REMOTE-origin")?;

    let final_snapshot = support::latest_snapshot(&remote_return.attachment)
        .map_err(|_| "final terminal snapshot synchronization failed".to_owned())?;
    if !safe_snapshot_contains(&final_snapshot, "LOCAL-origin")?
        || !safe_snapshot_contains(&final_snapshot, "REMOTE-origin")?
    {
        return Err("final authoritative screen lost cross-principal continuity".into());
    }
    other_controller
        .attachment
        .write_input(b"printf 'OTHER-STILL-%s\\n' \"$ZTERM_STEP8_OTHER\"\n")
        .map_err(support::display)?;
    wait_for_safe_marker(&other_controller.attachment, "OTHER-STILL-alive")?;

    let listed = fixture.service.list().map_err(support::display)?;
    assert_eq!(listed.len(), 2);
    assert!(
        listed
            .iter()
            .any(|summary| summary.session_id == created.session_id)
    );
    assert!(
        listed
            .iter()
            .any(|summary| summary.session_id == other.session_id)
    );
    remote_return.attachment.detach();
    other_controller.attachment.detach();
    fixture.service.shutdown().map_err(support::display)?;
    Ok(())
}

#[cfg(unix)]
fn wait_for_safe_marker(
    attachment: &zterm_daemon::session::SessionAttachment,
    marker: &str,
) -> Result<(), String> {
    let deadline = std::time::Instant::now() + support::DEADLINE;
    loop {
        let snapshot = support::latest_snapshot(attachment)
            .map_err(|_| "terminal snapshot synchronization failed".to_owned())?;
        if safe_snapshot_contains(&snapshot, marker)? {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err("expected terminal checkpoint was not reached before the deadline".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[cfg(unix)]
fn safe_snapshot_contains(
    snapshot: &zterm_core::terminal::TerminalSnapshot,
    marker: &str,
) -> Result<bool, String> {
    support::snapshot_text(snapshot)
        .map(|text| text.contains(marker))
        .map_err(|_| "terminal snapshot rendering failed".to_owned())
}
