use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::vte::ansi::{Color, NamedColor};
use zterm_core::terminal::{
    ActiveScreen, TerminalCell, TerminalColor, TerminalCursor, TerminalModes,
    TerminalMouseEncoding, TerminalMouseMode, TerminalScrollMetrics, TerminalSize, TerminalStyle,
    TerminalSurface, TerminalSurfaceRow,
};

use crate::MAX_CELL_TEXT_BYTES;
use crate::engine::AlacrittyEngine;

pub(crate) const CHECKPOINT_FORMAT_VERSION: u16 = 1;

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct InlineCellText {
    bytes: [u8; MAX_CELL_TEXT_BYTES],
    len: u8,
}

impl Default for InlineCellText {
    fn default() -> Self {
        Self {
            bytes: [0; MAX_CELL_TEXT_BYTES],
            len: 0,
        }
    }
}

impl InlineCellText {
    fn push(&mut self, character: char) -> bool {
        let mut encoded = [0; 4];
        let bytes = character.encode_utf8(&mut encoded).as_bytes();
        let start = usize::from(self.len);
        let Some(end) = start.checked_add(bytes.len()) else {
            return false;
        };
        let Some(destination) = self.bytes.get_mut(start..end) else {
            return false;
        };
        destination.copy_from_slice(bytes);
        self.len = u8::try_from(end).unwrap_or(u8::MAX);
        true
    }

    pub(crate) fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..usize::from(self.len)]).unwrap_or_default()
    }
}

#[derive(Clone, Default, Eq, PartialEq)]
pub(crate) struct ProjectedCell {
    pub(crate) text: InlineCellText,
    pub(crate) wide: bool,
    pub(crate) wide_continuation: bool,
    pub(crate) style: TerminalStyle,
}

impl ProjectedCell {
    pub(crate) fn to_public(&self) -> TerminalCell {
        TerminalCell {
            contents: self.text.as_str().to_owned(),
            wide: self.wide,
            wide_continuation: self.wide_continuation,
            style: self.style,
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct ProjectedRow {
    pub(crate) cells: Box<[ProjectedCell]>,
    pub(crate) wrapped: bool,
}

impl ProjectedRow {
    pub(crate) fn to_surface_row(&self) -> TerminalSurfaceRow {
        TerminalSurfaceRow {
            cells: self.cells.iter().map(ProjectedCell::to_public).collect(),
            wrapped: self.wrapped,
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct ProjectedScreen {
    pub(crate) version: u16,
    pub(crate) size: TerminalSize,
    pub(crate) active_screen: ActiveScreen,
    pub(crate) rows: Box<[ProjectedRow]>,
    pub(crate) cursor: TerminalCursor,
    pub(crate) modes: TerminalModes,
}

impl ProjectedScreen {
    pub(crate) fn retained_cell_capacity(&self) -> usize {
        self.rows.iter().map(|row| row.cells.len()).sum()
    }

    pub(crate) fn to_surface(
        &self,
        scroll_metrics: Option<TerminalScrollMetrics>,
    ) -> TerminalSurface {
        TerminalSurface {
            size: self.size,
            active_screen: self.active_screen,
            rows: self.rows.iter().map(ProjectedRow::to_surface_row).collect(),
            cursor: self.cursor,
            modes: self.modes,
            scroll_metrics,
        }
    }
}

pub(crate) fn project(engine: &AlacrittyEngine) -> ProjectedScreen {
    let term = engine.term();
    let grid = term.grid();
    let size = engine.size();
    let rows = (0..size.rows)
        .map(|row| project_row(engine, Line(i32::from(row))))
        .collect::<Vec<_>>()
        .into_boxed_slice();
    let point = grid.cursor.point;
    let row = usize::try_from(point.line.0)
        .unwrap_or_default()
        .min(usize::from(size.rows).saturating_sub(1));
    let column = point
        .column
        .0
        .min(usize::from(size.columns).saturating_sub(1));
    ProjectedScreen {
        version: CHECKPOINT_FORMAT_VERSION,
        size,
        active_screen: engine.active_screen(),
        rows,
        cursor: TerminalCursor {
            row: u16::try_from(row).unwrap_or(u16::MAX),
            column: u16::try_from(column).unwrap_or(u16::MAX),
            visible: term.mode().contains(TermMode::SHOW_CURSOR),
            style: terminal_style(&grid.cursor.template),
        },
        modes: terminal_modes(engine),
    }
}

pub(crate) fn project_row(engine: &AlacrittyEngine, line: Line) -> ProjectedRow {
    let grid = engine.term().grid();
    let cells = (0..grid.columns())
        .map(|column| projected_cell(&grid[line][Column(column)]))
        .collect::<Vec<_>>()
        .into_boxed_slice();
    let wrapped = grid[line]
        .last()
        .is_some_and(|cell| cell.flags.contains(Flags::WRAPLINE));
    ProjectedRow { cells, wrapped }
}

fn projected_cell(cell: &Cell) -> ProjectedCell {
    let style = terminal_style(cell);
    let wide = cell.flags.contains(Flags::WIDE_CHAR);
    let wide_continuation = cell.flags.contains(Flags::WIDE_CHAR_SPACER);
    let zerowidth = cell.zerowidth().unwrap_or_default();
    let default_blank = cell.c == ' '
        && zerowidth.is_empty()
        && style == TerminalStyle::default()
        && !wide
        && !wide_continuation;
    let mut text = InlineCellText::default();
    if !default_blank && !wide_continuation && !cell.c.is_control() {
        let _ = text.push(cell.c);
    }
    for character in zerowidth {
        if !text.push(*character) {
            break;
        }
    }
    ProjectedCell {
        text,
        wide,
        wide_continuation,
        style,
    }
}

fn terminal_style(cell: &Cell) -> TerminalStyle {
    TerminalStyle {
        foreground: terminal_color(cell.fg),
        background: terminal_color(cell.bg),
        bold: cell.flags.contains(Flags::BOLD),
        dim: cell.flags.contains(Flags::DIM),
        italic: cell.flags.contains(Flags::ITALIC),
        underline: cell.flags.intersects(Flags::ALL_UNDERLINES),
        inverse: cell.flags.contains(Flags::INVERSE),
    }
}

fn terminal_color(color: Color) -> TerminalColor {
    match color {
        Color::Indexed(index) => TerminalColor::Indexed(index),
        Color::Spec(rgb) => TerminalColor::Rgb(rgb.r, rgb.g, rgb.b),
        Color::Named(named) => named_color(named),
    }
}

fn named_color(color: NamedColor) -> TerminalColor {
    let index = match color {
        NamedColor::Black => Some(0),
        NamedColor::Red => Some(1),
        NamedColor::Green => Some(2),
        NamedColor::Yellow => Some(3),
        NamedColor::Blue => Some(4),
        NamedColor::Magenta => Some(5),
        NamedColor::Cyan => Some(6),
        NamedColor::White => Some(7),
        NamedColor::BrightBlack => Some(8),
        NamedColor::BrightRed => Some(9),
        NamedColor::BrightGreen => Some(10),
        NamedColor::BrightYellow => Some(11),
        NamedColor::BrightBlue => Some(12),
        NamedColor::BrightMagenta => Some(13),
        NamedColor::BrightCyan => Some(14),
        NamedColor::BrightWhite => Some(15),
        NamedColor::DimBlack => Some(0),
        NamedColor::DimRed => Some(1),
        NamedColor::DimGreen => Some(2),
        NamedColor::DimYellow => Some(3),
        NamedColor::DimBlue => Some(4),
        NamedColor::DimMagenta => Some(5),
        NamedColor::DimCyan => Some(6),
        NamedColor::DimWhite => Some(7),
        NamedColor::Foreground
        | NamedColor::Background
        | NamedColor::Cursor
        | NamedColor::BrightForeground
        | NamedColor::DimForeground => None,
    };
    index.map_or(TerminalColor::Default, TerminalColor::Indexed)
}

fn terminal_modes(engine: &AlacrittyEngine) -> TerminalModes {
    let mode = engine.term().mode();
    let mouse_mode = if mode.contains(TermMode::MOUSE_MOTION) {
        TerminalMouseMode::AnyMotion
    } else if mode.contains(TermMode::MOUSE_DRAG) {
        TerminalMouseMode::ButtonMotion
    } else if mode.contains(TermMode::MOUSE_REPORT_CLICK) {
        TerminalMouseMode::PressRelease
    } else if engine.legacy_x10_mouse() {
        TerminalMouseMode::Press
    } else {
        TerminalMouseMode::None
    };
    let mouse_encoding = if mode.contains(TermMode::SGR_MOUSE) {
        TerminalMouseEncoding::Sgr
    } else if mode.contains(TermMode::UTF8_MOUSE) {
        TerminalMouseEncoding::Utf8
    } else {
        TerminalMouseEncoding::Default
    };
    TerminalModes {
        application_keypad: mode.contains(TermMode::APP_KEYPAD),
        application_cursor: mode.contains(TermMode::APP_CURSOR),
        bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
        focus_reporting: mode.contains(TermMode::FOCUS_IN_OUT),
        alternate_scroll: mode.contains(TermMode::ALTERNATE_SCROLL),
        mouse_mode,
        mouse_encoding,
    }
}
