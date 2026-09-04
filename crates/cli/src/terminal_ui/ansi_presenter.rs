use std::collections::BTreeMap;
use std::io::{self, Write};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use zterm_core::terminal::{
    TerminalCell, TerminalClipboardWrite, TerminalKeyboardFlags, TerminalModes, TerminalStyle,
    TerminalColor,
};
use zterm_core::terminal_selection::{TerminalTextPoint, TerminalTextRange};

use super::{
    CliError, ComposedFrame, HOST_INPUT_CAPTURE, HOST_SYNC_BEGIN, HOST_SYNC_END,
    keyboard::desired_outer_keyboard_flags, normalize_composed_row, terminal_io,
    selection::{SelectionPresentation, SelectionSourceIdentity},
};

/// Sole active desktop writer and owner of the last successfully flushed frame.
#[derive(Clone, Default)]
pub(super) struct DesktopPresenter {
    pub(super) baseline: Option<ComposedFrame>,
    committed_input_modes: HostInputModes,
    selection: SelectionPresentation,
}

/// Exact child modes which change bytes produced by the physical outer
/// terminal. Mouse reporting and alternate-scroll remain Zterm-routed state
/// and are intentionally absent.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct HostInputModes {
    application_cursor: bool,
    application_keypad: bool,
    bracketed_paste: bool,
    focus_reporting: bool,
    keyboard_flags: TerminalKeyboardFlags,
}

impl HostInputModes {
    fn desired(child: TerminalModes, copy_ready: bool) -> Self {
        Self {
            application_cursor: child.application_cursor,
            application_keypad: child.application_keypad,
            bracketed_paste: child.bracketed_paste,
            focus_reporting: child.focus_reporting,
            keyboard_flags: desired_outer_keyboard_flags(child.keyboard_flags, copy_ready),
        }
    }

    fn terminal_modes(self) -> TerminalModes {
        TerminalModes {
            application_cursor: self.application_cursor,
            application_keypad: self.application_keypad,
            bracketed_paste: self.bracketed_paste,
            focus_reporting: self.focus_reporting,
            keyboard_flags: self.keyboard_flags,
            ..TerminalModes::default()
        }
    }
}

impl DesktopPresenter {
    pub(super) fn set_selection(
        &mut self,
        source: Option<SelectionSourceIdentity>,
        selection: Option<TerminalTextRange>,
        copy_ready: bool,
    ) {
        self.selection = SelectionPresentation::from_parts(source, selection, copy_ready);
    }

    pub(super) fn outer_keyboard_flags(
        &self,
        child: TerminalKeyboardFlags,
    ) -> TerminalKeyboardFlags {
        desired_outer_keyboard_flags(child, self.selection.copy_ready())
    }

    /// Returns the mode known to have reached the physical terminal. Desired
    /// selection state can move ahead of a paced frame, so input decoding must
    /// remain tied to this committed value until that frame is flushed.
    pub(super) fn presented_keyboard_flags(&self) -> TerminalKeyboardFlags {
        self.committed_input_modes.keyboard_flags
    }

    /// Synchronizes physical input encoding without repainting a retained
    /// history frame. Input semantics follow the latest child modes even while
    /// the visible semantic rows remain pinned.
    pub(super) fn sync_input_modes(
        &mut self,
        writer: &mut impl Write,
        child: TerminalModes,
        selection: SelectionPresentation,
    ) -> Result<bool, CliError> {
        let desired = HostInputModes::desired(child, selection.copy_ready());
        if self.committed_input_modes == desired {
            self.selection = selection;
            return Ok(false);
        }

        let mut sequence = Vec::with_capacity(64);
        write_changed_input_modes(&mut sequence, self.committed_input_modes, desired)
            .map_err(|error| terminal_io("compose terminal input modes", error))?;
        if let Err(error) = writer.write_all(&sequence) {
            self.baseline = None;
            return Err(terminal_io("synchronize terminal input modes", error));
        }
        if let Err(error) = writer.flush() {
            self.baseline = None;
            return Err(terminal_io("flush terminal input modes", error));
        }

        self.committed_input_modes = desired;
        self.selection = selection;
        if let Some(baseline) = &mut self.baseline {
            baseline.modes = desired.terminal_modes();
        }
        Ok(true)
    }

    pub(super) fn present(
        &mut self,
        writer: &mut impl Write,
        desired: ComposedFrame,
        source: Option<SelectionSourceIdentity>,
    ) -> Result<bool, CliError> {
        self.present_candidate(writer, desired, source, self.selection)
    }

    pub(super) fn present_candidate(
        &mut self,
        writer: &mut impl Write,
        mut desired: ComposedFrame,
        source: Option<SelectionSourceIdentity>,
        selection: SelectionPresentation,
    ) -> Result<bool, CliError> {
        if let Some(selection) = selection.range_for(source) {
            for (row, cells) in &mut desired.rows {
                for (column, cell) in cells.iter_mut().enumerate() {
                    let Ok(column) = u16::try_from(column) else {
                        continue;
                    };
                    if selection.contains(TerminalTextPoint::new(*row, column)) {
                        cell.style.inverse = !cell.style.inverse;
                    }
                }
            }
        }
        let desired_input_modes = HostInputModes::desired(
            desired.modes,
            selection.copy_ready_for(source),
        );
        // A committed frame retains only modes represented in the physical
        // terminal. Routed child mouse/alternate-scroll state must not create
        // a false presentation difference later.
        desired.modes = desired_input_modes.terminal_modes();
        if self.baseline.as_ref() == Some(&desired) {
            self.selection = selection;
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
        write_terminal_modes(&mut frame, desired_input_modes)
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
        self.committed_input_modes = desired_input_modes;
        self.selection = selection;
        self.baseline = Some(desired);
        Ok(true)
    }

    pub(super) fn write_clipboard(
        &mut self,
        writer: &mut impl Write,
        write: &TerminalClipboardWrite,
    ) -> Result<(), CliError> {
        let encoded = STANDARD.encode(write.as_str().as_bytes());
        let mut sequence = Vec::with_capacity(encoded.len().saturating_add(8));
        sequence.extend_from_slice(b"\x1b]52;c;");
        sequence.extend_from_slice(encoded.as_bytes());
        sequence.push(0x07);
        writer
            .write_all(&sequence)
            .map_err(|error| terminal_io("write terminal clipboard effect", error))?;
        writer
            .flush()
            .map_err(|error| terminal_io("flush terminal clipboard effect", error))
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

fn write_terminal_modes(writer: &mut impl Write, modes: HostInputModes) -> io::Result<()> {
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
    write_keyboard_mode(writer, modes.keyboard_flags)?;
    Ok(())
}

fn write_changed_input_modes(
    writer: &mut impl Write,
    before: HostInputModes,
    after: HostInputModes,
) -> io::Result<()> {
    if before.application_cursor != after.application_cursor {
        writer.write_all(if after.application_cursor {
            b"\x1b[?1h"
        } else {
            b"\x1b[?1l"
        })?;
    }
    if before.application_keypad != after.application_keypad {
        writer.write_all(if after.application_keypad {
            b"\x1b="
        } else {
            b"\x1b>"
        })?;
    }
    if before.bracketed_paste != after.bracketed_paste {
        writer.write_all(if after.bracketed_paste {
            b"\x1b[?2004h"
        } else {
            b"\x1b[?2004l"
        })?;
    }
    if before.focus_reporting != after.focus_reporting {
        writer.write_all(if after.focus_reporting {
            b"\x1b[?1004h"
        } else {
            b"\x1b[?1004l"
        })?;
    }
    if before.keyboard_flags != after.keyboard_flags {
        write_keyboard_mode(writer, after.keyboard_flags)?;
    }
    Ok(())
}

fn write_keyboard_mode(
    writer: &mut impl Write,
    flags: TerminalKeyboardFlags,
) -> io::Result<()> {
    write!(writer, "\x1b[={}u", flags.bits())
}
