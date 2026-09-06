"""Replay unmodified production cursor blocks without changing product code.

This diagnosis probe is deliberately narrower than the future integrated CLI
regression: it compiles the current composition decision and ANSI cursor writer
with minimal surrounding types. It does not exercise transport or native IME.
"""

from pathlib import Path
import subprocess
import tempfile


ROOT = next(
    parent
    for parent in Path(__file__).resolve().parents
    if (parent / "crates/cli/src/terminal_ui/composition.rs").is_file()
)
composition = (ROOT / "crates/cli/src/terminal_ui/composition.rs").read_text()
presenter = (ROOT / "crates/cli/src/terminal_ui/ansi_presenter.rs").read_text()
cursor_start = composition.index("        let cursor = if transport_state")
cursor_end = composition.index("        Ok(Self {", cursor_start)
cursor_block = composition[cursor_start:cursor_end]
writer_start = presenter.index("        write!(\n            frame,\n            \"\\x1b[{};{}H\"")
writer_end = presenter.index("        frame.extend_from_slice(HOST_INPUT_CAPTURE);", writer_start)
writer_block = presenter[writer_start:writer_end]

source = r'''
use std::io::{self, Write};
#[derive(Clone, Copy, Debug, Default)]
struct TerminalStyle;
#[derive(Clone, Copy, Debug)]
struct ComposedCursor { row: u16, column: u16, visible: bool, style: TerminalStyle }
struct Surface { cursor: ComposedCursor }
struct Size { rows: u16, columns: u16 }
#[derive(Clone, Copy, PartialEq)]
enum TerminalViewTransportState { Active, Reconnecting }
fn terminal_io(_: &str, error: io::Error) -> io::Error { error }
fn compose(surface: &Surface, is_live: bool, transport_state: TerminalViewTransportState) -> ComposedCursor {
    let content_size = Size { rows: 40, columns: 100 };
''' + cursor_block + r'''
    cursor
}
fn emit(cursor: ComposedCursor) -> io::Result<Vec<u8>> {
    let desired = Surface { cursor };
    let mut frame = Vec::new();
''' + writer_block + r'''
    Ok(frame)
}
fn main() -> io::Result<()> {
    for (label, row, col, visible, is_live, state) in [
        ("visible", 34, 27, true, true, TerminalViewTransportState::Active),
        ("hidden", 34, 27, false, true, TerminalViewTransportState::Active),
        ("hidden-moved", 35, 31, false, true, TerminalViewTransportState::Active),
        ("history", 34, 27, true, false, TerminalViewTransportState::Active),
        ("reconnecting", 34, 27, true, true, TerminalViewTransportState::Reconnecting),
    ] {
        let cursor = ComposedCursor { row, column: col, visible, style: TerminalStyle };
        let result = compose(&Surface { cursor }, is_live, state);
        println!("{label}: child=({row},{col},{visible}) outer=({},{},{}) ANSI={:?}",
            result.row, result.column, result.visible, String::from_utf8_lossy(&emit(result)?));
    }
    Ok(())
}
'''
with tempfile.TemporaryDirectory(prefix="zterm-ime-probe-") as directory:
    path = Path(directory)
    (path / "probe.rs").write_text(source)
    subprocess.run(["rustc", "--edition=2024", str(path / "probe.rs"), "-o", str(path / "probe")], check=True)
    subprocess.run([str(path / "probe")], check=True)
