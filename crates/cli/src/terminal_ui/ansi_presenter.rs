use std::collections::BTreeMap;
use std::io::{self, Write};

use zterm_core::terminal::{TerminalCell, TerminalColor, TerminalModes, TerminalStyle};

use super::{
    CliError, ComposedFrame, HOST_INPUT_CAPTURE, HOST_SYNC_BEGIN, HOST_SYNC_END, normalize_composed_row,
    terminal_io,
};

/// Sole active desktop writer and owner of the last successfully flushed frame.
#[derive(Clone, Default)]
pub(super) struct DesktopPresenter {
    pub(super) baseline: Option<ComposedFrame>,
}

impl DesktopPresenter {
    pub(super) fn present(
        &mut self,
        writer: &mut impl Write,
        desired: ComposedFrame,
    ) -> Result<bool, CliError> {
        if self.baseline.as_ref() == Some(&desired) {
            return Ok(false);
        }
        let baseline = self.baseline.as_ref().filter(|baseline| {
            baseline.physical_size == desired.physical_size && baseline.layout == desired.layout
        });
        let mut frame = Vec::new();
        frame.extend_from_slice(HOST_SYNC_BEGIN);
        if baseline.is_none() {
            frame.extend_from_slice(b"\x1b[0m\x1b[2J");
        }
        let mut row_indices = BTreeMap::new();
        if let Some(baseline) = baseline {
            for row in baseline.rows.keys() {
                row_indices.insert(*row, ());
            }
        }
        for row in desired.rows.keys() {
            row_indices.insert(*row, ());
        }
        for row in row_indices.keys().copied() {
            let before = baseline.and_then(|baseline| baseline.rows.get(&row));
            let after = desired.rows.get(&row);
            if before == after {
                continue;
            }
            let width = before
                .map(Vec::len)
                .unwrap_or(0)
                .max(after.map(Vec::len).unwrap_or(0))
                .min(usize::from(desired.physical_size.columns));
            if width == 0 || row >= desired.physical_size.rows {
                continue;
            }
            let mut complete = after.cloned().unwrap_or_default();
            normalize_composed_row(&mut complete, width);
            let mut previous = before.cloned().unwrap_or_default();
            normalize_composed_row(&mut previous, width);
            for (start, end) in semantic_dirty_runs(&previous, &complete) {
                write!(frame, "\x1b[{};{}H", row + 1, start + 1)
                    .map_err(|error| terminal_io("compose semantic terminal row", error))?;
                encode_semantic_row(&mut frame, &complete[start..end])
                    .map_err(|error| terminal_io("compose semantic terminal row", error))?;
            }
        }
        write_terminal_modes(&mut frame, desired.modes)
            .map_err(|error| terminal_io("compose semantic terminal modes", error))?;
        write_style(&mut frame, desired.cursor.style)
            .map_err(|error| terminal_io("compose semantic terminal cursor", error))?;
        write!(
            frame,
            "\x1b[{};{}H",
            desired.cursor.row.saturating_add(1),
            desired.cursor.column.saturating_add(1)
        )
        .map_err(|error| terminal_io("compose semantic terminal cursor", error))?;
        frame.extend_from_slice(if desired.cursor.visible {
            b"\x1b[?25h"
        } else {
            b"\x1b[?25l"
        });
        frame.extend_from_slice(HOST_INPUT_CAPTURE);
        frame.extend_from_slice(HOST_SYNC_END);
        if let Err(error) = writer.write_all(&frame) {
            self.baseline = None;
            let _ = writer.write_all(HOST_SYNC_END);
            let _ = writer.flush();
            return Err(terminal_io("present semantic terminal frame", error));
        }
        if let Err(error) = writer.flush() {
            self.baseline = None;
            let _ = writer.write_all(HOST_SYNC_END);
            let _ = writer.flush();
            return Err(terminal_io("present semantic terminal frame", error));
        }
        self.baseline = Some(desired);
        Ok(true)
    }
}

pub(super) fn semantic_dirty_runs(
    before: &[TerminalCell],
    after: &[TerminalCell],
) -> Vec<(usize, usize)> {
    debug_assert_eq!(before.len(), after.len());
    let mut dirty = before
        .iter()
        .zip(after)
        .map(|(before, after)| before != after)
        .collect::<Vec<_>>();
    for index in 0..dirty.len() {
        if !dirty[index] {
            continue;
        }
        for row in [before, after] {
            if row[index].wide {
                if let Some(next) = dirty.get_mut(index + 1) {
                    *next = true;
                }
            } else if row[index].wide_continuation && index > 0 {
                dirty[index - 1] = true;
            }
        }
    }
    let mut runs = Vec::new();
    let mut index = 0;
    while index < dirty.len() {
        if !dirty[index] {
            index += 1;
            continue;
        }
        let start = index;
        while index < dirty.len() && dirty[index] {
            index += 1;
        }
        runs.push((start, index));
    }
    runs
}

fn encode_semantic_row(writer: &mut impl Write, row: &[TerminalCell]) -> io::Result<()> {
    let mut style = None;
    let mut column = 0;
    while column < row.len() {
        let cell = &row[column];
        if cell.wide_continuation {
            column += 1;
            continue;
        }
        if style != Some(cell.style) {
            write_style(writer, cell.style)?;
            style = Some(cell.style);
        }
        if cell.contents.is_empty() {
            writer.write_all(b" ")?;
        } else {
            writer.write_all(cell.contents.as_bytes())?;
        }
        column += if cell.wide { 2 } else { 1 };
    }
    writer.write_all(b"\x1b[0m")
}

fn write_style(writer: &mut impl Write, style: TerminalStyle) -> io::Result<()> {
    let mut parameters = vec!["0".to_owned()];
    if style.bold {
        parameters.push("1".to_owned());
    }
    if style.dim {
        parameters.push("2".to_owned());
    }
    if style.italic {
        parameters.push("3".to_owned());
    }
    if style.underline {
        parameters.push("4".to_owned());
    }
    if style.inverse {
        parameters.push("7".to_owned());
    }
    push_semantic_color(&mut parameters, style.foreground, true);
    push_semantic_color(&mut parameters, style.background, false);
    write!(writer, "\x1b[{}m", parameters.join(";"))
}

fn push_semantic_color(parameters: &mut Vec<String>, color: TerminalColor, foreground: bool) {
    let (default, prefix) = if foreground {
        ("39", "38")
    } else {
        ("49", "48")
    };
    match color {
        TerminalColor::Default => parameters.push(default.to_owned()),
        TerminalColor::Indexed(index) => parameters.push(format!("{prefix};5;{index}")),
        TerminalColor::Rgb(red, green, blue) => {
            parameters.push(format!("{prefix};2;{red};{green};{blue}"));
        }
    }
}

fn write_terminal_modes(writer: &mut impl Write, modes: TerminalModes) -> io::Result<()> {
    writer.write_all(
        b"\x1b[?1l\x1b>\x1b[?2004l\x1b[?1004l\x1b[?1007l\x1b[?9l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1005l\x1b[?1006l",
    )?;
    if modes.application_cursor {
        writer.write_all(b"\x1b[?1h")?;
    }
    if modes.application_keypad {
        writer.write_all(b"\x1b=")?;
    }
    if modes.bracketed_paste {
        writer.write_all(b"\x1b[?2004h")?;
    }
    if modes.focus_reporting {
        writer.write_all(b"\x1b[?1004h")?;
    }
    Ok(())
}
