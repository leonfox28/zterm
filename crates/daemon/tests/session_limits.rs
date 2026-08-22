//! Session-count, viewport, projection, and resize rollback integration tests.

#[cfg(unix)]
#[path = "support/session_fixture.rs"]
mod support;

#[cfg(unix)]
#[test]
fn session_viewport_and_projection_limits_fail_without_mutation() -> Result<(), String> {
    use zterm_core::terminal::{TerminalModel, TerminalSize};
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

    let mut tight = ResourceLimits::default();
    let initial = TerminalSize::new(tight.no_controller_rows, tight.no_controller_columns);
    tight.aggregate_cell_projection_bytes = TerminalModel::project_resources(
        TerminalSize::new(tight.max_viewport_rows, tight.max_viewport_columns),
        tight.recent_history_rows,
    )
    .map_err(support::display)?
    .estimated_cell_storage_bytes;
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
    let aggregate = fixture
        .service
        .create(
            fixture.principal,
            fixture.op(2),
            SessionName::new("aggregate-overflow").expect("fixture name"),
            None,
            Some(initial),
        )
        .expect_err("aggregate projection must reject a second session");
    assert_eq!(aggregate.kind(), DomainErrorKind::ResourceExhausted);
    assert_eq!(fixture.service.list().map_err(support::display)?.len(), 1);
    let before = fixture.service.list().map_err(support::display)?[0].clone();
    let oversized = attached
        .attachment
        .resize(TerminalSize::new(81, 240))
        .expect_err("oversized viewport");
    assert_eq!(oversized.kind(), DomainErrorKind::ResourceExhausted);
    let after = fixture.service.list().map_err(support::display)?[0].clone();
    assert_eq!(
        before, after,
        "failed resize must retain state and reservation"
    );
    let oversized_attach = match fixture.service.prepare_attach(
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
