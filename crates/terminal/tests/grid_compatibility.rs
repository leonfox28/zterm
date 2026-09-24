//! Basic control compatibility without bypassing terminal resource budgets.
use zterm_core::terminal::{TerminalSize, TerminalSurfaceDeltaResult};
use zterm_core::terminal_selection::{TerminalTextPoint, TerminalTextRange};
use zterm_terminal::{MAX_CELL_TEXT_BYTES, MAX_REPEAT_COUNT, TerminalModel};

fn model() -> TerminalModel {
    TerminalModel::new(TerminalSize::new(4, 24), 4).expect("model")
}

#[test]
fn repeat_uses_the_normal_character_path_across_chunks_and_character_sets() {
    for (input, expanded) in [
        ("A\x1b[3b", "AAAA"),
        ("界\x1b[2b", "界界界"),
        ("e\u{301}\x1b[2b", "e\u{301}\u{301}\u{301}"),
        ("\x1b(0q\x1b[2b", "\x1b(0qqq"),
        ("A\x1b[b\x1b[0b", "AAA"),
    ] {
        let mut expected = model();
        expected.ingest(expanded.as_bytes()).expect("expanded");
        for chunk in [1, 3, input.len()] {
            let mut actual = model();
            for bytes in input.as_bytes().chunks(chunk) {
                actual.ingest(bytes).expect("repeat");
            }
            assert_eq!(
                actual.snapshot().surface.rows,
                expected.snapshot().surface.rows
            );
            assert_eq!(
                actual.snapshot().surface.cursor,
                expected.snapshot().surface.cursor
            );
        }
    }
}

#[test]
fn repeat_bounds_and_invalid_predecessors_do_not_reuse_stale_text() {
    let mut m = model();
    m.ingest(b"\x1b[3bA\x1b[4097b\x1b[2;3b\x1b[?2b")
        .expect("bounded");
    assert_eq!(m.snapshot().surface.cursor.column, 1);
    for invalid in [b"\xff".as_slice(), b"\xe2"] {
        let mut m = model();
        m.ingest(b"A").expect("base");
        m.ingest(invalid).expect("invalid UTF-8");
        m.ingest(b"\x1b[0m").expect("flush partial");
        let rows = m.snapshot().surface.rows;
        m.ingest(b"\x1b[3b").expect("no stale repeat");
        assert_eq!(m.snapshot().surface.rows, rows);
    }
    m.ingest(b"\x1bc\x1b[3b").expect("reset");
    assert_eq!(m.snapshot().surface.cursor.column, 0);

    m.ingest("e\u{301}".as_bytes()).expect("base combining");
    m.ingest(format!("\x1b[{MAX_REPEAT_COUNT}b").as_bytes())
        .expect("bounded combining repeat");
    assert!(m.snapshot().surface.rows[0].cells[0].contents.len() <= MAX_CELL_TEXT_BYTES);
    m.snapshot().validate().expect("valid bounded cells");
}

#[test]
fn ansi_restore_and_kitty_keyboard_controls_remain_distinct() {
    let mut m = model();
    let update = m
        .ingest(b"\x1b[2;3H\x1b[s\x1b[4;8H\x1b[u\x1b[>1u\x1b[?u")
        .expect("restore and keyboard");
    assert_eq!(update.replies, b"\x1b[?1u");
    assert_eq!(
        (
            m.snapshot().surface.cursor.row,
            m.snapshot().surface.cursor.column
        ),
        (1, 2)
    );
    m.ingest(b"\x1b[<u").expect("keyboard pop");
    assert_eq!(m.snapshot().surface.modes.keyboard_flags.bits(), 0);
}

#[test]
fn strike_and_conceal_survive_projection_delta_and_explicit_copy() {
    let mut m = model();
    let snapshot = m.snapshot();
    let checkpoint = m.checkpoint();
    m.ingest(b"\x1b[8;9mX\x1b[28mY\x1b[29mZ\x1b[8;9m\x1b[0mW")
        .expect("styles");
    let expected = m.snapshot();
    let row = &expected.surface.rows[0];
    assert!(row.cells[0].style.strike && row.cells[0].style.conceal);
    assert!(row.cells[1].style.strike && !row.cells[1].style.conceal);
    assert!(!row.cells[2].style.strike && !row.cells[2].style.conceal);
    assert!(!row.cells[3].style.strike && !row.cells[3].style.conceal);
    let TerminalSurfaceDeltaResult::Delta(delta) = m.delta_or_resync(&checkpoint) else {
        panic!("delta")
    };
    assert_eq!(
        delta
            .candidate(snapshot.revision, &snapshot.surface)
            .expect("candidate"),
        expected.surface
    );
    let copied = TerminalTextRange::new(TerminalTextPoint::new(0, 0), TerminalTextPoint::new(0, 3))
        .extract(&expected.surface.rows)
        .expect("explicit copy");
    assert_eq!(copied.as_str(), "XYZW");
}
