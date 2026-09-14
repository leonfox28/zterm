//! Actual host resize ordering; no daemon, PTY, network or user state.
use zterm_core::terminal::{ActiveScreen, TerminalSize, TerminalSurfaceSnapshot};
use zterm_terminal::TerminalModel;

fn rows(snapshot: &TerminalSurfaceSnapshot) -> Vec<String> {
    snapshot.surface.rows.iter().map(|row| {
        row.cells.iter().map(|cell| if cell.contents.is_empty() { " " } else {
            cell.contents.as_str()
        }).collect::<String>().trim_end().to_owned()
    }).collect()
}

fn main() {
    for screen in [ActiveScreen::Main, ActiveScreen::Alternate] {
        for cursor in [0, 3, 21, 23] {
            let mut model = TerminalModel::new(TerminalSize::new(24, 16), 32).unwrap();
            let mut initial = String::new();
            if screen == ActiveScreen::Alternate { initial.push_str("\x1b[?1049h"); }
            for row in 0..24 { initial.push_str(&format!("\x1b[{};1HROW {row:02}", row + 1)); }
            initial.push_str(&format!("\x1b[{};1H", cursor + 1));
            model.ingest(initial.as_bytes()).unwrap();
            let original = rows(&model.snapshot());
            model.resize(TerminalSize::new(12, 16)).unwrap();
            let resized = model.snapshot();
            let scrolled = (cursor + 1usize).saturating_sub(12);
            assert_eq!(resized.surface.active_screen, screen);
            assert_eq!(rows(&resized), original[scrolled..scrolled + 12]);
            assert_eq!(resized.surface.cursor.row as usize, cursor - scrolled);
            // Local top-edge clamping is independent from host caret-preserving resize.
            let local_shift = (-12i32).max(-(cursor as i32));
            println!("{screen:?} cursor={cursor}: local shift={local_shift}, host shift=-{scrolled}, host rows={}..{}",
                scrolled, scrolled + 11);
            model.ingest(b"\x1b[?2026h\x1b[2J").unwrap();
            assert_eq!(rows(&model.snapshot()), rows(&resized));
            assert_eq!(model.snapshot().surface.cursor, resized.surface.cursor);
            model.ingest(b"\x1b[?2026l").unwrap();
            assert!(rows(&model.snapshot()).iter().all(String::is_empty));

            // Separately verify normal shell-like resize with no child redraw.
            let mut model = TerminalModel::new(TerminalSize::new(24, 16), 32).unwrap();
            model.ingest(initial.as_bytes()).unwrap();
            model.resize(TerminalSize::new(12, 16)).unwrap();
            model.resize(TerminalSize::new(24, 16)).unwrap();
            let grown = model.snapshot();
            let from_history = if screen == ActiveScreen::Main { scrolled } else { 0 };
            let mut expected = original[scrolled - from_history..scrolled + 12].to_vec();
            expected.resize(24, String::new());
            assert_eq!(rows(&grown), expected);
            assert_eq!(grown.surface.cursor.row as usize, cursor - scrolled + from_history);
            println!("  growth restores {from_history} history rows, appends {} blanks", 12 - from_history);
        }
    }
}
