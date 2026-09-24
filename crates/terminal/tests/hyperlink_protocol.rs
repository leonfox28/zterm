//! Hyperlinks remain bounded semantic content across projection and retention.
use std::sync::Arc;
use zterm_core::terminal::*;
use zterm_terminal::TerminalModel;

#[test]
fn links_are_chunk_invariant_shared_and_preserved_in_deltas_and_history() {
    let bytes =
        "\x1b]8;id=doc;https://example.com/文档\x1b\\界ab\x1b]8;;\x1b\\plain\r\n2\r\n3".as_bytes();
    let mut expected = None;
    for chunk in [1, 7, bytes.len()] {
        let mut model = TerminalModel::new(TerminalSize::new(2, 20), 4).expect("model");
        let before = model.snapshot();
        let checkpoint = model.checkpoint();
        for input in bytes.chunks(chunk) {
            model.ingest(input).expect("OSC 8");
        }
        let snapshot = model.snapshot();
        let TerminalSurfaceDeltaResult::Delta(delta) = model.delta_or_resync(&checkpoint) else {
            panic!("delta")
        };
        assert_eq!(
            delta
                .candidate(before.revision, &before.surface)
                .expect("delta"),
            snapshot.surface
        );
        let metrics = snapshot.surface.scroll_metrics.expect("metrics");
        let query = TerminalHistoryWindowQuery {
            anchor: TerminalHistoryWindowAnchor {
                epoch: metrics.epoch,
                revision: snapshot.revision,
                max_offset_from_bottom: metrics.max_offset_from_bottom,
                viewport: snapshot.surface.size,
            },
            target_offset_from_bottom: 1,
            older_margin_rows: 0,
            newer_margin_rows: 0,
        };
        let Some(TerminalSurfaceHistoryWindowResult::Frame(history)) = model.history_window(query)
        else {
            panic!("history")
        };
        let row = &history.rows[0];
        let link = row.cells[0].hyperlink.as_ref().expect("retained target");
        assert_eq!(link.id(), "doc");
        assert_eq!(link.uri(), "https://example.com/%E6%96%87%E6%A1%A3");
        assert!(Arc::ptr_eq(
            link,
            row.cells[2].hyperlink.as_ref().expect("shared target")
        ));
        assert!(row.cells[4].hyperlink.is_none());
        if let Some(expected) = &expected {
            assert_eq!(&history.rows, expected)
        } else {
            expected = Some(history.rows);
        }
    }
}

#[test]
fn invalid_openers_close_links_and_automatic_identities_are_model_local() {
    let input = b"\x1b]8;;https://example.com\x07A\x1b]8;;file:///remote\x07B\x1b]8;;https://example.com\x07C\x1b]8;;\x07D";
    let mut one = TerminalModel::new(TerminalSize::new(2, 20), 0).expect("model");
    let mut two = TerminalModel::new(TerminalSize::new(2, 20), 0).expect("model");
    one.ingest(input).expect("links");
    for byte in input {
        two.ingest(&[*byte]).expect("split links");
    }
    let rows = one.snapshot().surface.rows;
    assert_eq!(rows, two.snapshot().surface.rows);
    assert!(rows[0].cells[0].hyperlink.is_some());
    assert!(rows[0].cells[1].hyperlink.is_none());
    assert!(rows[0].cells[2].hyperlink.is_some());
    assert!(rows[0].cells[3].hyperlink.is_none());
    assert_ne!(rows[0].cells[0].hyperlink, rows[0].cells[2].hyperlink);
    assert!(!format!("{rows:?}").contains("example.com"));
}

#[test]
fn identities_are_bounded_across_screens_and_saved_templates_and_are_reclaimed() {
    let mut model = TerminalModel::new(TerminalSize::new(40, 40), 0).expect("model");
    // Main saved template pins one link even though it has printed no cells.
    model
        .ingest(b"\x1b]8;id=saved;https://example.com/saved\x07\x1b7\x1b]8;;\x07\x1b[?47h")
        .expect("saved main");
    let mut input = String::new();
    for id in 0..MAX_TERMINAL_HYPERLINKS - 1 {
        input.push_str(&format!("\x1b]8;id={id};https://example.com/{id}\x07X"));
    }
    input.push_str("\x1b]8;id=overflow;https://example.com/overflow\x07Z");
    model.ingest(input.as_bytes()).expect("bounded links");
    let snapshot = model.snapshot();
    assert_eq!(
        snapshot
            .surface
            .rows
            .iter()
            .flat_map(|r| &r.cells)
            .filter(|c| c.hyperlink.is_some())
            .count(),
        MAX_TERMINAL_HYPERLINKS - 1
    );
    model
        .ingest(b"\x1b[2J\x1b[H\x1b]8;id=fresh;https://example.com/fresh\x07F\x1b]8;;\x07")
        .expect("reclaim");
    assert_eq!(
        model.snapshot().surface.rows[0].cells[0]
            .hyperlink
            .as_ref()
            .expect("reclaimed quota")
            .id(),
        "fresh"
    );
    model
        .ingest(b"\x1b[?47l\x1b8S")
        .expect("restore saved link");
    assert_eq!(
        model.snapshot().surface.rows[0].cells[0]
            .hyperlink
            .as_ref()
            .expect("saved template retained")
            .id(),
        "saved"
    );
    model.ingest(b"\x1bcplain").expect("reset");
    assert!(
        model.snapshot().surface.rows[0]
            .cells
            .iter()
            .all(|c| c.hyperlink.is_none())
    );
}
