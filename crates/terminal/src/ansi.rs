use zterm_core::terminal::{
    ALTERNATE_SCREEN_SELECTION_ANSI, ActiveScreen, MAIN_SCREEN_SELECTION_ANSI, TerminalColor,
    TerminalModes, TerminalMouseEncoding, TerminalMouseMode, TerminalStyle,
};

use crate::projection::{ProjectedCell, ProjectedRow, ProjectedScreen};

const RESET: &[u8] = b"\x1b[0m";
const CLEAR_HOME: &[u8] = b"\x1b[2J\x1b[H";

pub(crate) fn encode_full(screen: &ProjectedScreen) -> Vec<u8> {
    let mut output = Vec::new();
    output.extend_from_slice(MAIN_SCREEN_SELECTION_ANSI);
    if screen.active_screen == ActiveScreen::Alternate {
        output.extend_from_slice(ALTERNATE_SCREEN_SELECTION_ANSI);
    }
    output.extend_from_slice(RESET);
    output.extend_from_slice(&controlled_mode_reset());
    output.extend_from_slice(CLEAR_HOME);
    for (index, row) in screen.rows.iter().enumerate() {
        push_cup(&mut output, index, 0);
        output.extend_from_slice(RESET);
        encode_row_content(&mut output, row);
    }
    restore_cursor_and_modes(&mut output, screen);
    output
}

pub(crate) fn encode_delta(baseline: &ProjectedScreen, latest: &ProjectedScreen) -> Vec<u8> {
    let mut output = Vec::new();
    for (index, (before, after)) in baseline.rows.iter().zip(&latest.rows).enumerate() {
        if before == after {
            continue;
        }
        push_cup(&mut output, index, 0);
        output.extend_from_slice(RESET);
        encode_row_content(&mut output, after);
        // Preserve the old row until its replacement bytes have arrived. EL0
        // then removes only a stale suffix and does not expose an empty row on
        // outer terminals which do not implement synchronized presentation.
        // Reset first so EL0 clears with the default background rather than
        // inheriting the final encoded cell's style.
        output.extend_from_slice(RESET);
        output.extend_from_slice(b"\x1b[K");
    }
    restore_cursor_and_modes(&mut output, latest);
    output
}

pub(crate) fn encode_history_row(row: &ProjectedRow) -> Vec<u8> {
    let mut output = RESET.to_vec();
    encode_row_content(&mut output, row);
    output.extend_from_slice(RESET);
    output
}

fn encode_row_content(output: &mut Vec<u8>, row: &ProjectedRow) {
    let end = if row.wrapped {
        row.cells.len()
    } else {
        row.cells
            .iter()
            .rposition(|cell| !cell.is_visually_empty_default())
            .map_or(0, |index| index + 1)
    };
    let mut active_style = TerminalStyle::default();
    for cell in row.cells.iter().take(end) {
        if cell.wide_continuation {
            continue;
        }
        if cell.style != active_style {
            push_style(output, cell.style);
            active_style = cell.style;
        }
        push_cell_text(output, cell);
    }
}

fn push_cell_text(output: &mut Vec<u8>, cell: &ProjectedCell) {
    let text = cell.text.as_str();
    if text.is_empty() {
        output.push(b' ');
        return;
    }
    for character in text.chars() {
        if character != '\u{1b}' && !character.is_control() {
            let mut buffer = [0; 4];
            output.extend_from_slice(character.encode_utf8(&mut buffer).as_bytes());
        }
    }
}

fn push_cup(output: &mut Vec<u8>, row: usize, column: usize) {
    output.extend_from_slice(format!("\x1b[{};{}H", row + 1, column + 1).as_bytes());
}

fn push_style(output: &mut Vec<u8>, style: TerminalStyle) {
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
    push_color(&mut parameters, true, style.foreground);
    push_color(&mut parameters, false, style.background);
    output.extend_from_slice(format!("\x1b[{}m", parameters.join(";")).as_bytes());
}

fn push_color(parameters: &mut Vec<String>, foreground: bool, color: TerminalColor) {
    let default = if foreground { "39" } else { "49" };
    let prefix = if foreground { "38" } else { "48" };
    match color {
        TerminalColor::Default => parameters.push(default.to_owned()),
        TerminalColor::Indexed(index) => parameters.push(format!("{prefix};5;{index}")),
        TerminalColor::Rgb(red, green, blue) => {
            parameters.push(format!("{prefix};2;{red};{green};{blue}"));
        }
    }
}

fn restore_cursor_and_modes(output: &mut Vec<u8>, screen: &ProjectedScreen) {
    push_style(output, screen.cursor.style);
    push_cup(
        output,
        usize::from(screen.cursor.row),
        usize::from(screen.cursor.column),
    );
    output.extend_from_slice(if screen.cursor.visible {
        b"\x1b[?25h"
    } else {
        b"\x1b[?25l"
    });
    output.extend_from_slice(&controlled_mode_reset());
    output.extend_from_slice(&enabled_modes(screen.modes));
}

fn controlled_mode_reset() -> Vec<u8> {
    b"\x1b[?1l\x1b>\x1b[?2004l\x1b[?1004l\x1b[?1007l\x1b[?9l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1005l\x1b[?1006l"
        .to_vec()
}

fn enabled_modes(modes: TerminalModes) -> Vec<u8> {
    let mut output = Vec::new();
    if modes.application_cursor {
        output.extend_from_slice(b"\x1b[?1h");
    }
    if modes.application_keypad {
        output.extend_from_slice(b"\x1b=");
    }
    if modes.bracketed_paste {
        output.extend_from_slice(b"\x1b[?2004h");
    }
    if modes.focus_reporting {
        output.extend_from_slice(b"\x1b[?1004h");
    }
    if modes.alternate_scroll {
        output.extend_from_slice(b"\x1b[?1007h");
    }
    match modes.mouse_mode {
        TerminalMouseMode::None => {}
        TerminalMouseMode::Press => output.extend_from_slice(b"\x1b[?9h"),
        TerminalMouseMode::PressRelease => output.extend_from_slice(b"\x1b[?1000h"),
        TerminalMouseMode::ButtonMotion => output.extend_from_slice(b"\x1b[?1002h"),
        TerminalMouseMode::AnyMotion => output.extend_from_slice(b"\x1b[?1003h"),
    }
    match modes.mouse_encoding {
        TerminalMouseEncoding::Default => {}
        TerminalMouseEncoding::Utf8 => output.extend_from_slice(b"\x1b[?1005h"),
        TerminalMouseEncoding::Sgr => output.extend_from_slice(b"\x1b[?1006h"),
    }
    output
}

#[cfg(test)]
pub(crate) fn uses_only_allowlisted_ansi(bytes: &[u8], screen_prefix: bool) -> bool {
    let mut index = 0;
    if screen_prefix {
        if !bytes.starts_with(MAIN_SCREEN_SELECTION_ANSI) {
            return false;
        }
        index += MAIN_SCREEN_SELECTION_ANSI.len();
        if bytes
            .get(index..)
            .is_some_and(|bytes| bytes.starts_with(ALTERNATE_SCREEN_SELECTION_ANSI))
        {
            index += ALTERNATE_SCREEN_SELECTION_ANSI.len();
        }
    }

    while index < bytes.len() {
        match bytes[index] {
            b'\r' | b'\n' => index += 1,
            0x00..=0x1a | 0x1c..=0x1f | 0x7f => return false,
            0x1b => {
                let Some(next) = bytes.get(index + 1).copied() else {
                    return false;
                };
                if matches!(next, b'=' | b'>') {
                    index += 2;
                    continue;
                }
                if next != b'[' {
                    return false;
                }
                let Some(relative_final) = bytes[index + 2..]
                    .iter()
                    .position(|byte| (0x40..=0x7e).contains(byte))
                else {
                    return false;
                };
                let final_index = index + 2 + relative_final;
                let body = &bytes[index + 2..final_index];
                if !allowlisted_csi(body, bytes[final_index]) {
                    return false;
                }
                index = final_index + 1;
            }
            _ => index += 1,
        }
    }
    true
}

#[cfg(test)]
fn allowlisted_csi(body: &[u8], final_byte: u8) -> bool {
    match final_byte {
        b'm' => {
            body.is_empty()
                || (body.first() == Some(&b'0')
                    && body
                        .iter()
                        .all(|byte| byte.is_ascii_digit() || *byte == b';'))
        }
        b'H' if body.is_empty() => true,
        b'H' => {
            let mut fields = body.split(|byte| *byte == b';');
            matches!((fields.next(), fields.next(), fields.next()), (Some(row), Some(column), None)
                if !row.is_empty()
                    && !column.is_empty()
                    && row.iter().all(u8::is_ascii_digit)
                    && column.iter().all(u8::is_ascii_digit))
        }
        b'J' => body == b"2",
        b'K' => body.is_empty() || body == b"2",
        b'h' | b'l' => matches!(
            body,
            b"?1"
                | b"?25"
                | b"?2004"
                | b"?1004"
                | b"?1007"
                | b"?9"
                | b"?1000"
                | b"?1002"
                | b"?1003"
                | b"?1005"
                | b"?1006"
        ),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ALTERNATE_SCREEN_SELECTION_ANSI, MAIN_SCREEN_SELECTION_ANSI, encode_delta,
        uses_only_allowlisted_ansi,
    };
    use crate::projection::{ProjectedCell, ProjectedRow, ProjectedScreen};
    use zterm_core::terminal::{
        ActiveScreen, TerminalColor, TerminalCursor, TerminalModes, TerminalSize, TerminalStyle,
    };

    #[test]
    fn vocabulary_guard_rejects_control_strings() {
        assert!(uses_only_allowlisted_ansi(b"\x1b[2Jplain", false));
        assert!(!uses_only_allowlisted_ansi(b"\x1b]8;;secret\x1b\\", false));
        assert!(!uses_only_allowlisted_ansi(b"\x1bPsecret\x1b\\", false));
        assert!(!uses_only_allowlisted_ansi(b"\x1b[?2026h", false));
        assert!(!uses_only_allowlisted_ansi(
            ALTERNATE_SCREEN_SELECTION_ANSI,
            false,
        ));
        assert!(!uses_only_allowlisted_ansi(
            ALTERNATE_SCREEN_SELECTION_ANSI,
            true,
        ));
        let mut full = MAIN_SCREEN_SELECTION_ANSI.to_vec();
        full.extend_from_slice(ALTERNATE_SCREEN_SELECTION_ANSI);
        full.extend_from_slice(b"\x1b[0m\x1b[2J\x1b[1;1Hplain");
        assert!(uses_only_allowlisted_ansi(&full, true));
    }

    #[test]
    fn delta_resets_style_after_content_before_clearing_a_stale_suffix() {
        fn screen(cells: Vec<ProjectedCell>) -> ProjectedScreen {
            ProjectedScreen {
                version: 1,
                size: TerminalSize::new(1, 2),
                active_screen: ActiveScreen::Main,
                rows: vec![ProjectedRow {
                    cells: cells.into_boxed_slice(),
                    wrapped: false,
                }]
                .into_boxed_slice(),
                cursor: TerminalCursor {
                    row: 0,
                    column: 0,
                    visible: true,
                    style: TerminalStyle::default(),
                },
                modes: TerminalModes::default(),
            }
        }

        let styled = |background| ProjectedCell {
            style: TerminalStyle {
                background: TerminalColor::Indexed(background),
                ..TerminalStyle::default()
            },
            ..ProjectedCell::default()
        };
        let baseline = screen(vec![styled(4), styled(4)]);
        let latest = screen(vec![styled(1), ProjectedCell::default()]);

        let delta = encode_delta(&baseline, &latest);
        let clear = delta
            .windows(b"\x1b[K".len())
            .position(|bytes| bytes == b"\x1b[K")
            .expect("changed row clears only its stale suffix");
        assert!(delta[..clear].ends_with(b"\x1b[0m"));
        assert!(!delta[..clear].ends_with(b"\x1b[2K"));
    }
}
