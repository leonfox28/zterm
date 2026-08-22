//! Snapshot, checkpoint, delta, and resource-boundary regression tests.

use zterm_core::terminal::{
    ActiveScreen, TerminalDeltaResult, TerminalError, TerminalModel, TerminalSize,
};

fn apply_snapshot(snapshot: &zterm_core::terminal::TerminalSnapshot) -> TerminalModel {
    let mut client = TerminalModel::new(snapshot.size, 64).expect("snapshot size is valid");
    if !snapshot.recent_history_ansi.is_empty() {
        client
            .ingest(&snapshot.recent_history_ansi)
            .expect("history replay succeeds");
    }
    client
        .ingest(&snapshot.screen_ansi)
        .expect("screen replay succeeds");
    client
}

fn apply_result(client: &mut TerminalModel, result: &TerminalDeltaResult) {
    match result {
        TerminalDeltaResult::Delta(delta) => {
            client.ingest(&delta.ansi).expect("delta replay succeeds");
        }
        TerminalDeltaResult::Resync(snapshot) => {
            *client = apply_snapshot(snapshot);
        }
    }
}

fn ingest_chunked(model: &mut TerminalModel, bytes: &[u8], widths: &[usize]) {
    let mut offset = 0;
    let mut index = 0;
    while offset < bytes.len() {
        let width = widths[index % widths.len()];
        let end = offset.saturating_add(width).min(bytes.len());
        model
            .ingest(&bytes[offset..end])
            .expect("chunked ingest succeeds");
        offset = end;
        index += 1;
    }
}

#[test]
fn snapshot_then_merged_delta_matches_latest_semantic_state() {
    for widths in [&[usize::MAX][..], &[1][..], &[4][..], &[1, 7, 3, 9, 2][..]] {
        let mut source =
            TerminalModel::new(TerminalSize::new(12, 60), 64).expect("source size is valid");
        ingest_chunked(
            &mut source,
            concat!(
                "\x1b[2J\x1b[H",
                "header\r\n",
                "\x1b[38;5;33mblue\x1b[0m ",
                "\x1b[48;2;9;8;7mRGB\x1b[0m 界e\u{301}",
                "\x1b[?2004h\x1b[?1006h\x1b[?1000h\x1b[?1004h",
            )
            .as_bytes(),
            widths,
        );
        let snapshot = source.snapshot();
        let checkpoint = source.checkpoint();
        let mut client = apply_snapshot(&snapshot);
        assert_eq!(client.state(), source.state(), "snapshot widths={widths:?}");

        ingest_chunked(
            &mut source,
            b"\x1b[4;8Hdelta-one\x1b[6;12Hdelta-two\x1b[?1004l",
            widths,
        );
        let result = source.delta_or_resync(&checkpoint);
        let TerminalDeltaResult::Delta(delta) = &result else {
            panic!("small merged update should be a delta, widths={widths:?}");
        };
        assert_eq!(delta.from_revision, snapshot.revision);
        assert_eq!(delta.to_revision, source.revision());
        apply_result(&mut client, &result);
        assert_eq!(client.state(), source.state(), "delta widths={widths:?}");
    }
}

#[test]
fn alternate_screen_transitions_restore_latest_state() {
    let mut source =
        TerminalModel::new(TerminalSize::new(10, 50), 32).expect("source size is valid");
    source.ingest(b"main-screen").expect("main screen ingests");
    let main_snapshot = source.snapshot();
    let main_checkpoint = source.checkpoint();
    let mut client = apply_snapshot(&main_snapshot);

    source
        .ingest(b"\x1b[?1049h\x1b[2J\x1b[Halternate-screen")
        .expect("alternate screen ingests");
    let alternate_result = source.delta_or_resync(&main_checkpoint);
    apply_result(&mut client, &alternate_result);
    assert_eq!(client.state(), source.state());
    assert_eq!(client.state().active_screen, ActiveScreen::Alternate);

    let alternate_snapshot = source.snapshot();
    let alternate_client = apply_snapshot(&alternate_snapshot);
    assert_eq!(alternate_client.state(), source.state());

    let alternate_checkpoint = source.checkpoint();
    source
        .ingest(b"\x1b[?1049l\x1b[Hmain-screen-restored")
        .expect("main screen restores");
    let main_result = source.delta_or_resync(&alternate_checkpoint);
    apply_result(&mut client, &main_result);
    assert_eq!(client.state(), source.state());
    assert_eq!(client.state().active_screen, ActiveScreen::Main);
}

#[test]
fn incompatible_or_larger_delta_chooses_full_resync() {
    let size = TerminalSize::new(4, 24);
    let mut resized = TerminalModel::new(size, 8).expect("source size is valid");
    resized.ingest(b"baseline").expect("baseline ingests");
    let before_resize = resized.checkpoint();
    resized
        .resize(TerminalSize::new(5, 30))
        .expect("resize succeeds");
    assert!(matches!(
        resized.delta_or_resync(&before_resize),
        TerminalDeltaResult::Resync(_)
    ));

    let mut future_source = TerminalModel::new(size, 8).expect("future source is valid");
    future_source.ingest(b"one").expect("first ingest succeeds");
    future_source
        .ingest(b"two")
        .expect("second ingest succeeds");
    let future_checkpoint = future_source.checkpoint();
    let mut older_source = TerminalModel::new(size, 8).expect("older source is valid");
    older_source.ingest(b"one").expect("older ingest succeeds");
    assert!(matches!(
        older_source.delta_or_resync(&future_checkpoint),
        TerminalDeltaResult::Resync(_)
    ));

    let large_size = TerminalSize::new(20, 24);
    let mut dense_baseline = TerminalModel::new(large_size, 8).expect("dense baseline is valid");
    for row in 1..=large_size.rows {
        dense_baseline
            .ingest(format!("\x1b[{row};1HXXXXXXXXXXXXXXXXXXXXXXXX").as_bytes())
            .expect("dense row ingests");
    }
    let dense_checkpoint = dense_baseline.checkpoint();
    let mut blank_latest = TerminalModel::new(large_size, 8).expect("blank latest is valid");
    for _ in 0..dense_baseline.revision().get() {
        blank_latest.ingest(b"\x1b[2J").expect("clear ingests");
    }
    let result = blank_latest.delta_or_resync(&dense_checkpoint);
    let TerminalDeltaResult::Resync(snapshot) = result else {
        let TerminalDeltaResult::Delta(delta) = result else {
            unreachable!("all terminal delta results are covered");
        };
        panic!(
            "a delta no smaller than the blank snapshot must resync: delta={}, snapshot={}",
            delta.ansi_payload_len(),
            blank_latest.snapshot().ansi_payload_len(),
        );
    };
    assert_eq!(snapshot.revision, blank_latest.revision());
}

#[test]
fn history_resources_and_revision_rules_are_bounded_and_typed() {
    let size = TerminalSize::new(3, 12);
    let mut model = TerminalModel::new(size, 4).expect("bounded model is valid");
    assert_eq!(model.revision().get(), 0);
    let empty = model.ingest(b"").expect("empty ingest is a no-op");
    assert_eq!(empty.revision.get(), 0);

    for line in 0..12 {
        model
            .ingest(format!("line-{line:02}\r\n").as_bytes())
            .expect("history line ingests");
    }
    let revision_before_resize = model.revision();
    model
        .resize(size)
        .expect("same-size resize is still ordered");
    assert_eq!(
        model.revision(),
        revision_before_resize
            .checked_next()
            .expect("test revision has room")
    );

    let snapshot = model.snapshot();
    assert!(!snapshot.recent_history_ansi.is_empty());
    assert!(snapshot.recent_history_ansi.len() <= 4 * usize::from(size.columns) + 64);
    let client = apply_snapshot(&snapshot);
    assert_eq!(client.state(), model.state());

    let projection = model.resource_projection();
    assert_eq!(projection.visible_cells_per_screen, 36);
    assert_eq!(projection.scrollback_capacity_cells, 48);
    assert_eq!(projection.total_cell_capacity, 120);
    assert!(projection.estimated_cell_storage_bytes >= projection.total_cell_capacity);

    assert_eq!(
        TerminalModel::new(TerminalSize::new(0, 12), 4).err(),
        Some(TerminalError::InvalidSize(TerminalSize::new(0, 12)))
    );
    assert_eq!(
        model.resize(TerminalSize::new(3, 0)),
        Err(TerminalError::InvalidSize(TerminalSize::new(3, 0)))
    );
    assert_eq!(
        TerminalModel::new(TerminalSize::new(1, 2), usize::MAX).err(),
        Some(TerminalError::ResourceProjectionOverflow {
            size: TerminalSize::new(1, 2),
            scrollback_rows: usize::MAX,
        })
    );
}
