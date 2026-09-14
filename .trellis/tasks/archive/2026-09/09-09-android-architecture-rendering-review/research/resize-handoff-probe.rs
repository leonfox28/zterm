//! Process-free reproduction using the actual host terminal engine.
use zterm_core::terminal::{TerminalSize, TerminalSurfaceSnapshot};
use zterm_terminal::TerminalModel;

fn rows(snapshot: &TerminalSurfaceSnapshot) -> Vec<String> {
    snapshot.surface.rows.iter().map(|row| {
        row.cells.iter().map(|cell| if cell.contents.is_empty() { " " } else { cell.contents.as_str() }).collect::<String>()
            .trim_end().to_owned()
    }).collect()
}

fn main() {
    let mut model = TerminalModel::new(TerminalSize::new(12, 16), 0).unwrap();
    let mut initial = String::from("\x1b[?1049h");
    for row in 0..12 { initial.push_str(&format!("\x1b[{};1HROW {row:02}", row + 1)); }
    initial.push_str("\x1b[11;1H"); // One footer row below the visible caret.
    model.ingest(initial.as_bytes()).unwrap();
    let before = model.snapshot();
    let moved = rows(&before).into_iter().skip(3).collect::<Vec<_>>();
    model.resize(TerminalSize::new(9, 16)).unwrap();
    let resized = model.snapshot();
    println!("local moved rows: {moved:?}");
    println!("resize-only rows: {:?}; cursor={}", rows(&resized), resized.surface.cursor.row);
    assert_eq!(rows(&resized), rows(&before)[2..11]);
    assert_ne!(rows(&resized), moved);

    model.ingest(b"\x1b[?2026h\x1b[2J").unwrap();
    assert_eq!(model.snapshot().surface, resized.surface, "marked clear retains the resize-only content");
    let mut repaint = String::new();
    for (row, content) in moved.iter().enumerate() {
        repaint.push_str(&format!("\x1b[{};1H{content}", row + 1));
    }
    repaint.push_str("\x1b[8;1H\x1b[?2026l");
    model.ingest(repaint.as_bytes()).unwrap();
    let repainted = model.snapshot();
    println!("TUI repainted rows: {:?}; cursor={}", rows(&repainted), repainted.surface.cursor.row);
    assert_eq!(rows(&repainted), moved);
    println!("Confirmed: resize-only publication moves every retained row DOWN one row before TUI redraw restores the locally moved presentation.");
}
