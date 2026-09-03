//! Semantic snapshot, checkpoint, delta, and resource-boundary regressions.

use zterm_core::terminal::{ActiveScreen, TerminalColor, TerminalSize, TerminalSurfaceDeltaResult};
use zterm_terminal::{TerminalError, TerminalModel};

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
fn snapshot_then_merged_delta_matches_latest_semantic_surface() {
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
        let checkpoint = source.checkpoint();
        let mut applied = source.snapshot();
        applied.validate().expect("initial snapshot is valid");

        ingest_chunked(
            &mut source,
            b"\x1b[4;8Hdelta-one\x1b[6;12Hdelta-two\x1b[?1004l",
            widths,
        );
        let TerminalSurfaceDeltaResult::Delta(delta) = source.delta_or_resync(&checkpoint) else {
            panic!("compatible geometry must produce semantic row patches");
        };
        delta
            .apply_to(applied.revision, &mut applied.surface)
            .expect("semantic delta applies transactionally");
        applied.revision = delta.to_revision;
        assert_eq!(applied, source.snapshot(), "chunk widths={widths:?}");
    }
}

#[test]
fn semantic_projection_preserves_rightmost_wide_and_styled_blank_cells() {
    let mut model = TerminalModel::new(TerminalSize::new(3, 8), 8).expect("model");
    model
        .ingest("\x1b[1;8H\x1b[31m#\x1b[2;6H界\x1b[3;8H\x1b[44m \x1b[0m".as_bytes())
        .expect("paint edge cases");
    let snapshot = model.snapshot();
    let rightmost = &snapshot.surface.rows[0].cells[7];
    assert_eq!(rightmost.contents, "#");
    assert_eq!(rightmost.style.foreground, TerminalColor::Indexed(1));
    assert!(snapshot.surface.rows[1].cells[5].wide);
    assert!(snapshot.surface.rows[1].cells[6].wide_continuation);
    let styled_blank = &snapshot.surface.rows[2].cells[7];
    assert_eq!(styled_blank.contents, " ");
    assert_eq!(styled_blank.style.background, TerminalColor::Indexed(4));

    let checkpoint = model.checkpoint();
    model
        .ingest(b"\x1b[1;8H\x1b[32m@")
        .expect("replace final column");
    let TerminalSurfaceDeltaResult::Delta(delta) = model.delta_or_resync(&checkpoint) else {
        panic!("same geometry must produce a delta");
    };
    assert_eq!(delta.row_patches.len(), 1);
    assert_eq!(delta.row_patches[0].replacement.cells[7].contents, "@");
}

#[test]
fn screen_and_size_transitions_require_complete_semantic_resync() {
    let mut source = TerminalModel::new(TerminalSize::new(6, 32), 32).expect("source");
    source.ingest(b"main-screen").expect("main screen");
    let main_checkpoint = source.checkpoint();
    source
        .ingest(b"\x1b[?1049h\x1b[2J\x1b[Halternate-screen")
        .expect("alternate screen");
    let TerminalSurfaceDeltaResult::Resync(alternate) = source.delta_or_resync(&main_checkpoint)
    else {
        panic!("screen change must resync");
    };
    assert_eq!(alternate.surface.active_screen, ActiveScreen::Alternate);

    let alternate_checkpoint = source.checkpoint();
    source.ingest(b"\x1b[?1049l").expect("restore main");
    assert!(matches!(
        source.delta_or_resync(&alternate_checkpoint),
        TerminalSurfaceDeltaResult::Resync(_)
    ));

    let before_resize = source.checkpoint();
    source
        .resize(TerminalSize::new(8, 40))
        .expect("resize succeeds");
    assert!(matches!(
        source.delta_or_resync(&before_resize),
        TerminalSurfaceDeltaResult::Resync(_)
    ));
}

#[test]
fn checkpoint_is_visible_only_and_revision_only_updates_are_preserved() {
    let size = TerminalSize::new(6, 32);
    let mut model = TerminalModel::new(size, 2_000).expect("model");
    for line in 0..2_100 {
        model
            .ingest(format!("history-{line:04}\r\n").as_bytes())
            .expect("history line");
    }
    let checkpoint = model.checkpoint();
    assert_eq!(
        checkpoint.retained_cell_capacity(),
        usize::from(size.rows) * usize::from(size.columns)
    );
    assert_eq!(checkpoint.retained_scrollback_rows(), 0);

    model.resize(size).expect("same-size resize is ordered");
    let TerminalSurfaceDeltaResult::Delta(delta) = model.delta_or_resync(&checkpoint) else {
        panic!("same surface should retain the revision edge");
    };
    assert!(delta.row_patches.is_empty());
    assert_eq!(delta.to_revision, model.revision());
}

#[test]
fn invalid_dimensions_are_rejected_before_mutation() {
    assert_eq!(
        TerminalModel::new(TerminalSize::new(0, 12), 4).err(),
        Some(TerminalError::InvalidSize(TerminalSize::new(0, 12)))
    );
    assert_eq!(
        TerminalModel::new(TerminalSize::new(1, 2), usize::MAX).err(),
        Some(TerminalError::AllocationOverflow {
            size: TerminalSize::new(1, 2),
            scrollback_rows: usize::MAX,
        })
    );
}
