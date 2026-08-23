//! Session registry lifecycle, main singleflight, and replay integration tests.

#[cfg(unix)]
#[path = "support/session_fixture.rs"]
mod support;

#[cfg(unix)]
mod unix {
    use std::sync::{Arc, Barrier};
    use std::thread;

    use zterm_core::{
        DomainErrorKind, ResourceLimits, SessionEndReason, SessionName, SessionSelector,
    };
    use zterm_daemon::session::AttachmentLifecycle;

    use super::support::{self, Fixture};

    #[test]
    fn main_is_singleflight_and_survives_detach_until_explicit_close() -> Result<(), String> {
        let fixture = Arc::new(Fixture::new(ResourceLimits::default())?);
        let barrier = Arc::new(Barrier::new(8));
        let mut threads = Vec::new();
        for _ in 0..8 {
            let fixture = Arc::clone(&fixture);
            let barrier = Arc::clone(&barrier);
            threads.push(thread::spawn(move || {
                barrier.wait();
                match fixture.service.prepare_attach(
                    fixture.principal,
                    None,
                    true,
                    true,
                    Some(support::default_size()),
                ) {
                    Ok(prepared) => Ok(Some(prepared.attachment.session_id())),
                    Err(error) if error.kind() == DomainErrorKind::SessionOccupied => Ok(None),
                    Err(error) => Err(support::display(error)),
                }
            }));
        }
        let mut ids = Vec::new();
        for thread in threads {
            if let Some(id) = thread.join().map_err(|_| "attach thread panicked")?? {
                ids.push(id);
            }
        }
        assert!(!ids.is_empty(), "one first attachment must succeed");
        assert!(ids.iter().all(|id| *id == ids[0]));
        let sessions = fixture.service.list().map_err(support::display)?;
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, ids[0]);

        let prepared = fixture
            .service
            .prepare_attach(
                fixture.principal,
                Some(SessionSelector::Id(ids[0])),
                false,
                false,
                None,
            )
            .map_err(support::display)?;
        support::activate(&prepared)?;
        prepared
            .attachment
            .write_input(b"sleep 0.05; printf 'DETACHED-COMPLETE\\n'\n")
            .map_err(support::display)?;
        prepared.attachment.detach();
        thread::sleep(std::time::Duration::from_millis(100));

        let reattached = fixture
            .service
            .prepare_attach(fixture.principal, None, true, false, None)
            .map_err(support::display)?;
        assert_eq!(reattached.attachment.session_id(), ids[0]);
        support::activate(&reattached)?;
        support::wait_for_text(&reattached.attachment, "DETACHED-COMPLETE")?;
        let mut lifecycle = reattached
            .attachment
            .lifecycle_watch()
            .map_err(support::display)?;

        let closed = fixture
            .service
            .close(fixture.principal, fixture.op(1), ids[0])
            .map_err(support::display)?;
        assert_eq!(closed.name, SessionName::main());
        support::wait_for_session_count(&fixture.service, 0)?;
        assert_eq!(
            lifecycle.borrow_and_update().clone(),
            AttachmentLifecycle::SessionEnded(SessionEndReason::ExplicitClose)
        );
        let replacement = fixture
            .service
            .prepare_attach(fixture.principal, None, true, false, None)
            .map_err(support::display)?;
        assert_ne!(replacement.attachment.session_id(), ids[0]);
        let mut replacement_lifecycle = replacement
            .attachment
            .lifecycle_watch()
            .map_err(support::display)?;
        fixture
            .service
            .close(
                fixture.principal,
                fixture.op(2),
                replacement.attachment.session_id(),
            )
            .map_err(support::display)?;
        assert_eq!(
            replacement
                .attachment
                .snapshot_applied(replacement.snapshot.revision)
                .expect_err("an acknowledgement cannot revive an ended attachment")
                .kind(),
            DomainErrorKind::SessionNotFound
        );
        assert_eq!(
            replacement_lifecycle.borrow_and_update().clone(),
            AttachmentLifecycle::SessionEnded(SessionEndReason::ExplicitClose)
        );
        fixture.service.shutdown().map_err(support::display)?;
        Ok(())
    }

    #[test]
    fn named_lifecycle_replay_and_invalid_cwd_leave_no_half_state() -> Result<(), String> {
        let fixture = Fixture::new(ResourceLimits::default())?;
        let reserved = fixture
            .service
            .create(
                fixture.principal,
                fixture.op(9),
                SessionName::main(),
                None,
                None,
            )
            .expect_err("normal create cannot claim the reserved main name");
        assert_eq!(reserved.kind(), DomainErrorKind::ReservedSessionName);
        let reserved_replay = fixture
            .service
            .create(
                fixture.principal,
                fixture.op(9),
                SessionName::main(),
                None,
                None,
            )
            .expect_err("reserved-name error must replay without creating a session");
        assert_eq!(reserved_replay, reserved);
        assert!(fixture.service.list().map_err(support::display)?.is_empty());

        let build = fixture.create(10, "build").map_err(support::display)?;
        let replayed = fixture
            .service
            .create(
                fixture.principal,
                fixture.op(10),
                SessionName::new("build").expect("fixture name"),
                None,
                None,
            )
            .map_err(support::display)?;
        assert_eq!(replayed, build);
        let docs = fixture.create(11, "docs").map_err(support::display)?;
        assert_eq!(fixture.service.list().map_err(support::display)?.len(), 2);

        let renamed = fixture
            .service
            .rename(
                fixture.principal,
                fixture.op(12),
                docs.session_id,
                SessionName::new("review").expect("fixture name"),
            )
            .map_err(support::display)?;
        assert_eq!(renamed.session_id, docs.session_id);
        assert_eq!(renamed.name.as_str(), "review");
        let rename_replay = fixture
            .service
            .rename(
                fixture.principal,
                fixture.op(12),
                docs.session_id,
                SessionName::new("review").expect("fixture name"),
            )
            .map_err(support::display)?;
        assert_eq!(rename_replay, renamed);

        let conflict = fixture
            .service
            .rename(
                fixture.principal,
                fixture.op(13),
                build.session_id,
                SessionName::new("review").expect("fixture name"),
            )
            .expect_err("conflicting rename must fail");
        assert_eq!(conflict.kind(), DomainErrorKind::SessionAlreadyExists);

        let missing = fixture.temp.path().join("missing");
        let error = fixture
            .service
            .create(
                fixture.principal,
                fixture.op(14),
                SessionName::new("invalid-cwd").expect("fixture name"),
                Some(missing),
                None,
            )
            .expect_err("missing cwd must fail before publication");
        assert_eq!(error.kind(), DomainErrorKind::InvalidWorkingDirectory);
        assert_eq!(fixture.service.list().map_err(support::display)?.len(), 2);

        let closed = fixture
            .service
            .close(fixture.principal, fixture.op(15), docs.session_id)
            .map_err(support::display)?;
        assert_eq!(closed.session_id, renamed.session_id);
        assert_eq!(closed.name, renamed.name);
        assert_eq!(closed.working_directory, renamed.working_directory);
        assert_eq!(closed.viewport, renamed.viewport);
        assert!(
            closed.revision >= renamed.revision,
            "terminal revision must not move backwards before close"
        );
        let close_replay = fixture
            .service
            .close(fixture.principal, fixture.op(15), docs.session_id)
            .map_err(support::display)?;
        assert_eq!(close_replay, closed);
        support::wait_for_session_count(&fixture.service, 1)?;
        let conflict_replay = fixture
            .service
            .rename(
                fixture.principal,
                fixture.op(13),
                build.session_id,
                SessionName::new("review").expect("fixture name"),
            )
            .expect_err("retained typed error must replay exactly");
        assert_eq!(conflict_replay, conflict);
        assert_eq!(
            fixture.service.list().map_err(support::display)?[0].name,
            SessionName::new("build").expect("fixture name")
        );

        let attached = fixture
            .service
            .prepare_attach(
                fixture.principal,
                Some(SessionSelector::Id(build.session_id)),
                false,
                false,
                None,
            )
            .map_err(support::display)?;
        support::activate(&attached)?;
        let mut lifecycle = attached
            .attachment
            .lifecycle_watch()
            .map_err(support::display)?;
        attached
            .attachment
            .write_input(b"exit\n")
            .map_err(support::display)?;
        support::wait_for_session_count(&fixture.service, 0)?;
        let ended = lifecycle.borrow_and_update().clone();
        assert!(
            matches!(
                ended,
                AttachmentLifecycle::SessionEnded(SessionEndReason::NaturalExit { .. })
            ),
            "unexpected lifecycle after natural exit: {ended:?}"
        );
        fixture.service.shutdown().map_err(support::display)?;
        Ok(())
    }
}
