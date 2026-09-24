//! Application title is bounded global metadata with the normal publication rules.
use zterm_core::terminal::{MAX_TITLE_BYTES, TerminalSize, TerminalSurfaceDeltaResult};
use zterm_terminal::TerminalModel;

#[test]
fn title_only_updates_round_trip_through_delta_and_reset_without_touching_rows() {
    let mut model = TerminalModel::new(TerminalSize::new(2, 20), 4).expect("model");
    model.ingest(b"content").expect("text");
    for input in ["\x1b]0;编辑器\x07", "\x1b]2;build\x1b\\", "\x1b]2;\x07"] {
        let before = model.snapshot();
        let checkpoint = model.checkpoint();
        model.ingest(input.as_bytes()).expect("title");
        let after = model.snapshot();
        assert_eq!(after.surface.rows, before.surface.rows);
        let TerminalSurfaceDeltaResult::Delta(delta) = model.delta_or_resync(&checkpoint) else {
            panic!("delta")
        };
        assert!(delta.row_patches.is_empty());
        assert_eq!(
            delta
                .candidate(before.revision, &before.surface)
                .expect("title-only delta"),
            after.surface
        );
    }
    model
        .ingest(b"\x1b]2;persist\x07\x1b[?1049h")
        .expect("alternate");
    assert_eq!(model.snapshot().surface.application_title, "persist");
    model.ingest(b"\x1b[?1049l").expect("main");
    assert_eq!(model.snapshot().surface.application_title, "persist");
    model.ingest(b"\x1bc").expect("RIS");
    assert!(model.snapshot().surface.application_title.is_empty());
}

#[test]
fn title_is_clean_bounded_and_chunk_invariant_including_invalid_utf8() {
    let input = [
        b"\x1b]2;\t\r".as_slice(),
        "界".repeat(86).as_bytes(),
        b"\x07",
    ]
    .concat();
    for chunk in [1, 2, 7, input.len()] {
        let mut model = TerminalModel::new(TerminalSize::new(2, 20), 0).expect("model");
        for bytes in input.chunks(chunk) {
            model.ingest(bytes).expect("split title");
        }
        let snapshot = model.snapshot();
        assert_eq!(snapshot.surface.application_title, "界".repeat(85));
        assert!(snapshot.surface.application_title.len() <= MAX_TITLE_BYTES);
        assert!(!format!("{snapshot:?}").contains('界'));
        model
            .ingest(b"\x1b]2;invalid\xff\xc3\x07")
            .expect("invalid UTF-8");
        assert_eq!(
            model.snapshot().surface.application_title,
            "invalid\u{fffd}\u{fffd}"
        );
    }
}

#[test]
fn synchronized_output_publishes_title_cursor_style_and_links_together() {
    let mut model = TerminalModel::new(TerminalSize::new(2, 20), 0).expect("model");
    model.ingest(b"\x1b]2;before\x07\x1b[?2026h").expect("hold");
    let frozen = model.snapshot();
    let checkpoint = model.checkpoint();
    let update = model.ingest(b"\x1b]2;after\x07\x1b[5 q\x1b[8;9m\x1b]8;id=held;https://example.com/held\x07X\x1b]8;;\x07\x1b[0m\x1b[18t")
        .expect("held protocol changes");
    assert_eq!(update.replies, b"\x1b[8;2;20t");
    assert_eq!(model.snapshot(), frozen);
    model.ingest(b"\x1b[?2026l").expect("publish");
    let published = model.snapshot();
    assert_eq!(published.surface.application_title, "after");
    assert_eq!(
        published.surface.cursor.presentation.shape,
        zterm_core::terminal::TerminalCursorShape::Beam
    );
    assert!(published.surface.cursor.presentation.blinking);
    let cell = &published.surface.rows[0].cells[0];
    assert!(cell.style.strike && cell.style.conceal);
    assert_eq!(
        cell.hyperlink.as_ref().expect("published target").uri(),
        "https://example.com/held"
    );
    let TerminalSurfaceDeltaResult::Delta(delta) = model.delta_or_resync(&checkpoint) else {
        panic!("delta")
    };
    assert_eq!(
        delta
            .candidate(frozen.revision, &frozen.surface)
            .expect("combined delta"),
        published.surface
    );
}
