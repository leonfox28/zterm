//! Publication boundaries must not depend on PTY read chunking.

use zterm_core::terminal::{
    ActiveScreen, TerminalHistoryWindowAnchor, TerminalHistoryWindowQuery, TerminalSize,
    TerminalSurfaceHistoryWindowResult, TerminalSurfaceSnapshot,
};
use zterm_terminal::TerminalModel;

fn text(snapshot: &TerminalSurfaceSnapshot) -> String {
    snapshot
        .surface
        .rows
        .iter()
        .flat_map(|row| &row.cells)
        .map(|cell| cell.contents.as_str())
        .collect()
}

fn query(snapshot: &TerminalSurfaceSnapshot) -> TerminalHistoryWindowQuery {
    let metrics = snapshot
        .surface
        .scroll_metrics
        .expect("main screen metrics");
    TerminalHistoryWindowQuery {
        anchor: TerminalHistoryWindowAnchor {
            epoch: metrics.epoch,
            revision: snapshot.revision,
            max_offset_from_bottom: metrics.max_offset_from_bottom,
            viewport: snapshot.surface.size,
        },
        target_offset_from_bottom: 0,
        older_margin_rows: 0,
        newer_margin_rows: 0,
    }
}

#[test]
fn held_reads_keep_exact_cells_colors_and_history_while_queries_progress() {
    let mut model = TerminalModel::new(TerminalSize::new(2, 20), 2).expect("model");
    model.ingest(b"old\r\nvisible\x1b[?2026h").expect("begin");
    let frozen = model.snapshot();
    let checkpoint = model.checkpoint();
    let request = query(&frozen);
    let update = model
        .ingest(b"\x1b[2J\x1b[Hnew\r\na\r\nb\r\nc\x1b]4;1;#123456\x07\x1b[?2026$p\x1b[5n")
        .expect("held redraw and queries");
    assert_eq!(update.replies, b"\x1b[?2026;1$y\x1b[0n");
    assert_eq!(model.snapshot(), frozen);
    assert_eq!(model.checkpoint().revision(), checkpoint.revision());
    assert_eq!(model.published_revision(), frozen.revision);
    assert!(model.revision() > frozen.revision);
    assert!(model.history_window(request).is_none());

    let update = model.ingest(b"\x1b[?2026l\x1b[?2026$p").expect("end");
    assert_eq!(update.replies, b"\x1b[?2026;2$y");
    let complete = model.snapshot();
    complete.validate().expect("coherent published metadata");
    assert_ne!(complete.surface.colors, frozen.surface.colors);
    assert_eq!(complete.revision, model.revision());
    assert!(model.history_window(request).is_some());
}

#[test]
fn complete_a_then_partial_b_publishes_a_and_services_boundary_reads() {
    for chunk_size in [1, 3, usize::MAX] {
        let mut model = TerminalModel::new(TerminalSize::new(2, 20), 4).expect("model");
        let request = query(&model.snapshot());
        let mut saw_a_history = false;
        let input = b"\x1b[?2026hA-complete\x1b[?2026l\x1b[?2026h\x1b[2J\x1b[HB-partial";
        for bytes in input.chunks(chunk_size) {
            model
                .ingest_observing(bytes, |eligible| {
                    if let Some(TerminalSurfaceHistoryWindowResult::Frame(frame)) =
                        eligible.history_window(request)
                    {
                        let contents: String = frame
                            .rows
                            .iter()
                            .flat_map(|row| &row.cells)
                            .map(|cell| cell.contents.as_str())
                            .collect();
                        saw_a_history |= contents.contains("A-complete");
                        assert!(!contents.contains("B-partial"));
                    }
                })
                .expect("ordered chunks");
        }
        assert!(saw_a_history);
        let frozen = model.snapshot();
        assert!(text(&frozen).contains("A-complete"));
        assert!(!text(&frozen).contains("B-partial"));
        model.ingest(b"\x9b?2026l").expect("C1 end");
        assert!(text(&model.snapshot()).contains("B-partial"));
        assert!(model.snapshot().revision > frozen.revision);
    }
}

#[test]
fn combined_modes_are_inside_the_batch_and_recovery_is_bounded() {
    let mut model = TerminalModel::new(TerminalSize::new(2, 20), 4).expect("model");
    model
        .ingest(b"main\x1b[?1049;2026hALT")
        .expect("combined begin");
    assert_eq!(model.snapshot().surface.active_screen, ActiveScreen::Main);
    assert!(text(&model.snapshot()).contains("main"));
    let deadline = model.presentation_deadline().expect("hard deadline");
    model.ingest(b"\x1b[?2026h").expect("repeated begin");
    assert_eq!(model.presentation_deadline(), Some(deadline));
    assert!(model.expire_synchronized_output(deadline));
    assert_eq!(
        model.snapshot().surface.active_screen,
        ActiveScreen::Alternate
    );
    assert!(text(&model.snapshot()).contains("ALT"));
    assert_eq!(
        model
            .ingest(b"\x1b[?2026$p")
            .expect("query after timeout")
            .replies,
        b"\x1b[?2026;2$y"
    );

    model
        .ingest(b"\x1b[?2026hresize")
        .expect("begin before resize");
    model
        .resize(TerminalSize::new(3, 20))
        .expect("resize recovery");
    assert!(model.presentation_deadline().is_none());
    model.snapshot().validate().expect("valid resized surface");
    model.ingest(b"\x1b[?2026h\x1bc").expect("reset recovery");
    assert!(model.presentation_deadline().is_none());
    assert!(text(&model.snapshot()).trim().is_empty());
}
