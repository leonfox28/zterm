//! Read-only review probe against the repository's actual public terminal APIs.
//! This checks allocation identity, not timing or physical frame rate.
use zterm_core::terminal::{TerminalSize, TerminalSurfaceDeltaResult};
use zterm_terminal::TerminalModel;

fn main() {
    let mut model = TerminalModel::new(TerminalSize::new(53, 56), 1000).unwrap();
    let mut scene = String::new();
    for row in 1..=53 {
        scene.push_str(&format!("\x1b[{row};1H{}", "x".repeat(56)));
    }
    model.ingest(scene.as_bytes()).unwrap();
    let baseline = model.snapshot();
    let checkpoint = model.checkpoint();
    model.ingest(b"\x1b[1;1H").unwrap();
    let TerminalSurfaceDeltaResult::Delta(delta) = model.delta_or_resync(&checkpoint) else {
        panic!("same-size cursor movement should produce a delta");
    };
    assert!(delta.row_patches.is_empty());
    let candidate = delta
        .candidate(baseline.revision, &baseline.surface)
        .unwrap();
    assert_eq!(candidate, model.snapshot().surface);
    assert_eq!(candidate.rows, baseline.surface.rows);

    let copied_rows = baseline
        .surface
        .rows
        .iter()
        .zip(&candidate.rows)
        .filter(|(before, after)| before.cells.as_ptr() != after.cells.as_ptr())
        .count();
    let copied_texts = baseline
        .surface
        .rows
        .iter()
        .zip(&candidate.rows)
        .flat_map(|(before, after)| before.cells.iter().zip(&after.cells))
        .filter(|(before, after)| {
            !before.contents.is_empty() && before.contents.as_ptr() != after.contents.as_ptr()
        })
        .count();
    assert_eq!(copied_rows, 53);
    assert_eq!(copied_texts, 53 * 56);
    println!(
        "cursor-only: row_patches={}, equal_rows={}, copied_row_buffers={}, copied_nonempty_text_buffers={}",
        delta.row_patches.len(),
        candidate.rows.len(),
        copied_rows,
        copied_texts
    );

    let checkpoint = model.checkpoint();
    model.ingest(b"Y").unwrap();
    let TerminalSurfaceDeltaResult::Delta(delta) = model.delta_or_resync(&checkpoint) else {
        panic!("one changed cell should produce a delta");
    };
    assert_eq!(delta.row_patches.len(), 1);
    assert_eq!(delta.row_patches[0].replacement.cells.len(), 56);
    println!(
        "one-cell change: row_patches={}, replacement_cells={}",
        delta.row_patches.len(),
        delta.row_patches[0].replacement.cells.len()
    );
}
