//! Declared caret presentation is revisioned state, not a transient callback.
use zterm_core::terminal::{TerminalCursorShape, TerminalSize, TerminalSurfaceDeltaResult};
use zterm_terminal::TerminalModel;

#[test]
fn cursor_shapes_blinking_and_resets_survive_deltas() {
    let mut model = TerminalModel::new(TerminalSize::new(2, 10), 0).expect("model");
    for (code, shape, blinking) in [
        (1, TerminalCursorShape::Block, true),
        (2, TerminalCursorShape::Block, false),
        (3, TerminalCursorShape::Underline, true),
        (4, TerminalCursorShape::Underline, false),
        (5, TerminalCursorShape::Beam, true),
        (6, TerminalCursorShape::Beam, false),
        (0, TerminalCursorShape::Block, false),
    ] {
        let before = model.snapshot();
        let checkpoint = model.checkpoint();
        model
            .ingest(format!("\x1b[{code} q").as_bytes())
            .expect("cursor");
        let after = model.snapshot();
        assert_eq!(after.surface.cursor.presentation.shape, shape);
        assert_eq!(after.surface.cursor.presentation.blinking, blinking);
        let TerminalSurfaceDeltaResult::Delta(delta) = model.delta_or_resync(&checkpoint) else {
            panic!("delta")
        };
        assert!(delta.row_patches.is_empty());
        assert_eq!(
            delta
                .candidate(before.revision, &before.surface)
                .expect("cursor-only delta"),
            after.surface
        );
    }
    assert_eq!(
        model
            .ingest(b"\x1b[?12h\x1b[?12$p")
            .expect("blink query")
            .replies,
        b"\x1b[?12;1$y"
    );
    assert_eq!(
        model
            .ingest(b"\x1b[?12l\x1b[?12$p")
            .expect("steady query")
            .replies,
        b"\x1b[?12;2$y"
    );
    model.ingest(b"\x1b[5 q\x1b[?25l").expect("hidden beam");
    assert!(!model.snapshot().surface.cursor.visible);
    model.ingest(b"\x1bc").expect("RIS");
    assert_eq!(
        model.snapshot().surface.cursor.presentation,
        Default::default()
    );
    assert!(model.snapshot().surface.cursor.visible);
}

#[test]
fn held_cursor_changes_publish_only_at_the_existing_boundary() {
    let mut model = TerminalModel::new(TerminalSize::new(2, 10), 0).expect("model");
    model.ingest(b"\x1b[?2026h").expect("hold");
    let frozen = model.snapshot();
    model.ingest(b"\x1b[6 q").expect("beam");
    assert_eq!(model.snapshot(), frozen);
    model.ingest(b"\x1b[?2026l").expect("publish");
    assert_eq!(
        model.snapshot().surface.cursor.presentation.shape,
        TerminalCursorShape::Beam
    );
}
