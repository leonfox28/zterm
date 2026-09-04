//! Session-count, viewport, and resize rollback integration tests.

#[cfg(unix)]
#[path = "support/session_fixture.rs"]
mod support;

#[cfg(unix)]
#[test]
fn session_count_and_viewport_limits_fail_without_mutation() -> Result<(), String> {
    use zterm_core::terminal::TerminalSize;
    use zterm_core::{DomainErrorKind, ResourceLimits, SessionName, SessionSelector};

    let fixture = support::Fixture::new(ResourceLimits::default())?;
    for index in 0..8 {
        fixture
            .create(
                u64::try_from(index + 1).map_err(support::display)?,
                &format!("s{index}"),
            )
            .map_err(support::display)?;
    }
    assert_eq!(fixture.service.list().map_err(support::display)?.len(), 8);
    let ninth = fixture
        .create(20, "ninth")
        .expect_err("ninth session must fail admission");
    assert_eq!(ninth.kind(), DomainErrorKind::ResourceExhausted);
    assert_eq!(fixture.service.list().map_err(support::display)?.len(), 8);
    fixture.service.shutdown().map_err(support::display)?;

    let tight = ResourceLimits::default();
    let initial = TerminalSize::new(tight.no_controller_rows, tight.no_controller_columns);
    let fixture = support::Fixture::new(tight)?;
    let session = fixture
        .service
        .create(
            fixture.principal,
            fixture.op(1),
            SessionName::new("bounded").expect("fixture name"),
            None,
            Some(initial),
        )
        .map_err(support::display)?;
    let attached = fixture
        .service
        .prepare_attach(
            fixture.principal,
            Some(SessionSelector::Id(session.session_id)),
            false,
            false,
            None,
        )
        .map_err(support::display)?;
    support::activate(&attached)?;
    attached
        .attachment
        .resize(TerminalSize::new(80, 240))
        .map_err(support::display)?;
    let second = fixture
        .service
        .create(
            fixture.principal,
            fixture.op(2),
            SessionName::new("second-session").expect("fixture name"),
            None,
            Some(initial),
        )
        .map_err(support::display)?;
    assert_ne!(second.session_id, session.session_id);
    assert_eq!(fixture.service.list().map_err(support::display)?.len(), 2);
    let before = fixture
        .service
        .list()
        .map_err(support::display)?
        .into_iter()
        .find(|summary| summary.session_id == session.session_id)
        .ok_or_else(|| "resized session disappeared".to_owned())?;
    let oversized = attached
        .attachment
        .resize(TerminalSize::new(81, 240))
        .expect_err("oversized viewport");
    assert_eq!(oversized.kind(), DomainErrorKind::ResourceExhausted);
    let after = fixture
        .service
        .list()
        .map_err(support::display)?
        .into_iter()
        .find(|summary| summary.session_id == session.session_id)
        .ok_or_else(|| "resized session disappeared".to_owned())?;
    assert_eq!(
        before, after,
        "failed resize must retain terminal and viewport state"
    );
    let oversized_attach = match fixture.service.prepare_attach(
        fixture.principal,
        Some(SessionSelector::Id(session.session_id)),
        false,
        true,
        Some(TerminalSize::new(81, 240)),
    ) {
        Ok(_) => return Err("an oversized attachment viewport was ignored".into()),
        Err(error) => error,
    };
    assert_eq!(oversized_attach.kind(), DomainErrorKind::ResourceExhausted);
    fixture.service.shutdown().map_err(support::display)?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn current_controller_resize_survives_an_in_flight_snapshot() -> Result<(), String> {
    use zterm_core::terminal::TerminalSize;
    use zterm_core::{ResourceLimits, SessionSelector};
    use zterm_daemon::session::AttachmentUpdate;

    let fixture = support::Fixture::new(ResourceLimits::default())?;
    let session = fixture.create(1, "resize-sync").map_err(support::display)?;
    let attached = fixture
        .service
        .prepare_attach(
            fixture.principal,
            Some(SessionSelector::Id(session.session_id)),
            false,
            false,
            None,
        )
        .map_err(support::display)?;
    support::activate(&attached)?;

    let in_flight = attached
        .attachment
        .sync_latest(attached.snapshot.revision)
        .map_err(support::display)?;
    let resized = TerminalSize::new(31, 97);
    attached
        .attachment
        .resize(resized)
        .map_err(support::display)?;
    assert_eq!(
        fixture.service.list().map_err(support::display)?[0].viewport,
        resized
    );
    assert!(
        attached
            .attachment
            .snapshot_applied(in_flight.revision)
            .map_err(support::display)?
            .is_none()
    );
    let Some(AttachmentUpdate::Snapshot(replacement)) = attached
        .attachment
        .next_update()
        .map_err(support::display)?
    else {
        return Err("resize during synchronization did not require one latest snapshot".into());
    };
    assert_eq!(replacement.surface.size, resized);
    assert!(
        attached
            .attachment
            .snapshot_applied(replacement.revision)
            .map_err(support::display)?
            .is_none()
    );
    fixture.service.shutdown().map_err(support::display)?;
    Ok(())
}
