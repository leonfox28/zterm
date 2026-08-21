//! Host-authoritative terminal state and reconnect snapshots.
//!
//! The current implementation uses `vt100` internally, but every public type
//! is owned by zterm. The Foundation Gate reports VT100 primary device
//! attributes with the Advanced Video Option (`CSI ? 1;2 c`); it does not yet
//! assign a richer `TERM` value such as `xterm-256color`.

use std::{fmt, mem};

const PRIMARY_DEVICE_ATTRIBUTES_REPLY: &[u8] = b"\x1b[?1;2c";
const DEVICE_STATUS_OK_REPLY: &[u8] = b"\x1b[0n";
const MAIN_SCREEN: &[u8] = b"\x1b[?1049l";
const ALTERNATE_SCREEN: &[u8] = b"\x1b[?1049h";
const FOCUS_REPORTING_ON: &[u8] = b"\x1b[?1004h";
const FOCUS_REPORTING_OFF: &[u8] = b"\x1b[?1004l";

/// Maximum number of side events returned by one terminal update.
pub const MAX_SIDE_EVENTS_PER_UPDATE: usize = 32;

/// Maximum number of source bytes retained for a title or icon-name event.
pub const MAX_TITLE_BYTES: usize = 256;

/// Terminal viewport size in character cells.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalSize {
    /// Number of visible rows.
    pub rows: u16,
    /// Number of visible columns.
    pub columns: u16,
}

impl TerminalSize {
    /// Creates a viewport size. [`TerminalModel::new`] and
    /// [`TerminalModel::resize`] reject zero dimensions.
    #[must_use]
    pub const fn new(rows: u16, columns: u16) -> Self {
        Self { rows, columns }
    }
}

/// The currently visible terminal screen.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActiveScreen {
    /// The standard screen with bounded scrollback.
    Main,
    /// The alternate full-screen application screen.
    Alternate,
}

/// A terminal color independent of the private parser implementation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TerminalColor {
    /// The terminal's default color.
    #[default]
    Default,
    /// An indexed palette color.
    Indexed(u8),
    /// A true-color RGB value.
    Rgb(u8, u8, u8),
}

/// Drawing attributes for a cell or the active cursor.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TerminalStyle {
    /// Foreground color.
    pub foreground: TerminalColor,
    /// Background color.
    pub background: TerminalColor,
    /// Bold intensity is active.
    pub bold: bool,
    /// Dim intensity is active.
    pub dim: bool,
    /// Italic rendering is active.
    pub italic: bool,
    /// Underline rendering is active.
    pub underline: bool,
    /// Foreground and background are inverted.
    pub inverse: bool,
}

/// Semantic content of one visible terminal cell.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TerminalCell {
    /// Text held by the cell, including any combining characters.
    pub contents: String,
    /// Whether the cell starts a double-width character.
    pub wide: bool,
    /// Whether the cell is the continuation of a double-width character.
    pub wide_continuation: bool,
    /// Drawing attributes for the cell.
    pub style: TerminalStyle,
}

/// Semantic cursor state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalCursor {
    /// Zero-based cursor row.
    pub row: u16,
    /// Zero-based cursor column.
    pub column: u16,
    /// Whether the cursor is visible.
    pub visible: bool,
    /// Drawing attributes used for subsequently printed cells.
    pub style: TerminalStyle,
}

/// Mouse event mode requested by the hosted application.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TerminalMouseMode {
    /// Mouse reporting is disabled.
    #[default]
    None,
    /// Report button presses only.
    Press,
    /// Report button presses and releases.
    PressRelease,
    /// Report motion while a button is held.
    ButtonMotion,
    /// Report all mouse motion.
    AnyMotion,
}

/// Mouse event encoding requested by the hosted application.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TerminalMouseEncoding {
    /// Legacy single-byte encoding.
    #[default]
    Default,
    /// UTF-8 extended encoding.
    Utf8,
    /// SGR encoding.
    Sgr,
}

/// Input and presentation modes needed when attaching a controller.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TerminalModes {
    /// Application keypad mode is active.
    pub application_keypad: bool,
    /// Application cursor-key mode is active.
    pub application_cursor: bool,
    /// Bracketed paste mode is active.
    pub bracketed_paste: bool,
    /// Focus events should be reported to the hosted application.
    pub focus_reporting: bool,
    /// Mouse event reporting mode.
    pub mouse_mode: TerminalMouseMode,
    /// Mouse event encoding.
    pub mouse_encoding: TerminalMouseEncoding,
}

/// Zterm-owned semantic projection used to compare terminal states.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalState {
    /// Viewport size.
    pub size: TerminalSize,
    /// Currently visible screen.
    pub active_screen: ActiveScreen,
    /// Current cursor state.
    pub cursor: TerminalCursor,
    /// Input modes requested by the hosted application.
    pub modes: TerminalModes,
    /// Visible cells in row-major order.
    pub cells: Vec<TerminalCell>,
}

/// A side effect which zterm deliberately refuses to execute.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RejectedEffect {
    /// The application attempted to write the system clipboard.
    ClipboardWrite,
    /// The application requested clipboard contents.
    ClipboardRead,
}

/// A parser input class that was safely contained rather than forwarded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnsupportedSequenceKind {
    /// An unsupported printable character.
    Character,
    /// An unsupported control byte.
    Control,
    /// An unsupported escape sequence.
    Escape,
    /// An unsupported CSI sequence.
    Csi,
    /// An unsupported OSC sequence.
    Osc,
}

/// A bounded, non-rendering event produced while ingesting terminal output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerminalSideEvent {
    /// An audible bell was requested.
    AudibleBell,
    /// A visual bell was requested.
    VisualBell,
    /// The hosted application requested a viewport size.
    ResizeRequested(TerminalSize),
    /// The window title changed. The retained source bytes are bounded.
    TitleChanged {
        /// UTF-8-lossy title text.
        title: String,
        /// Whether source bytes were truncated.
        truncated: bool,
    },
    /// The window icon name changed. The retained source bytes are bounded.
    IconNameChanged {
        /// UTF-8-lossy icon name.
        icon_name: String,
        /// Whether source bytes were truncated.
        truncated: bool,
    },
    /// A security-sensitive host effect was refused.
    EffectRejected(RejectedEffect),
    /// An unsupported sequence was contained without forwarding its payload.
    UnsupportedSequence(UnsupportedSequenceKind),
    /// Additional side events were dropped to preserve the per-update bound.
    EventsDropped {
        /// Number of events omitted from this update.
        count: u64,
    },
}

/// Output produced by one ordered ingest or resize operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalUpdate {
    /// Model revision after the operation.
    pub revision: u64,
    /// Controlled bytes which must be written back to the hosted PTY.
    pub replies: Vec<u8>,
    /// Bounded non-rendering side events.
    pub events: Vec<TerminalSideEvent>,
}

/// Structural projection of fixed cell slots reserved by a terminal model.
///
/// This does not claim an RSS bound: parser state, row/container overhead,
/// snapshots, and transient workload allocations are intentionally left for
/// the Foundation resource measurements.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalResourceProjection {
    /// Cells in one visible screen.
    pub visible_cells_per_screen: usize,
    /// Configured main-screen scrollback capacity in cells.
    pub scrollback_capacity_cells: usize,
    /// Main, alternate, and scrollback cell capacity combined.
    pub total_cell_capacity: usize,
    /// Inline bytes for the fixed cell capacity, excluding parser,
    /// row/container, snapshot, and transient workload overhead.
    pub estimated_cell_storage_bytes: usize,
}

/// Full reconnect state at a specific revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalSnapshot {
    /// Revision represented by the snapshot.
    pub revision: u64,
    /// Viewport size represented by the snapshot.
    pub size: TerminalSize,
    /// Screen selected by the snapshot.
    pub active_screen: ActiveScreen,
    /// ANSI bytes which restore the current visible screen and modes.
    ///
    /// Apply these after [`Self::recent_history_ansi`].
    pub screen_ansi: Vec<u8>,
    /// Bounded standard scrollback encoded as an ANSI-compatible text stream.
    ///
    /// Apply these before [`Self::screen_ansi`] so replay leaves the current
    /// visible state authoritative.
    pub recent_history_ansi: Vec<u8>,
    /// Input modes represented by the snapshot.
    pub modes: TerminalModes,
}

impl TerminalSnapshot {
    /// Returns the number of ANSI payload bytes in this snapshot.
    #[must_use]
    pub fn ansi_payload_len(&self) -> usize {
        self.screen_ansi.len() + self.recent_history_ansi.len()
    }
}

/// A merged current-screen delta from one checkpoint to the latest revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalDelta {
    /// Checkpoint revision used as the baseline.
    pub from_revision: u64,
    /// Latest model revision represented by the delta.
    pub to_revision: u64,
    /// Viewport size represented by the delta.
    pub size: TerminalSize,
    /// Screen selected after applying the delta.
    pub active_screen: ActiveScreen,
    /// ANSI bytes which update the current visible terminal state.
    pub ansi: Vec<u8>,
    /// Input modes represented after applying the delta.
    pub modes: TerminalModes,
}

impl TerminalDelta {
    /// Returns the number of ANSI payload bytes in this delta.
    #[must_use]
    pub fn ansi_payload_len(&self) -> usize {
        self.ansi.len()
    }
}

/// Result of comparing a checkpoint with the latest terminal state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerminalDeltaResult {
    /// A smaller compatible delta can be applied to the checkpoint state.
    Delta(TerminalDelta),
    /// The checkpoint is incompatible or a full snapshot is no larger.
    Resync(TerminalSnapshot),
}

/// Opaque baseline for producing one merged latest-state delta.
#[derive(Clone)]
pub struct TerminalCheckpoint {
    revision: u64,
    size: TerminalSize,
    active_screen: ActiveScreen,
    focus_reporting: bool,
    screen: vt100::Screen,
}

impl fmt::Debug for TerminalCheckpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalCheckpoint")
            .field("revision", &self.revision)
            .field("size", &self.size)
            .field("active_screen", &self.active_screen)
            .finish_non_exhaustive()
    }
}

/// Errors produced at the terminal model boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerminalError {
    /// A viewport dimension was zero.
    InvalidSize(TerminalSize),
    /// The monotonically increasing revision reached `u64::MAX`.
    RevisionOverflow,
    /// A resource projection overflowed the target platform's `usize`.
    ResourceProjectionOverflow {
        /// Viewport which could not be projected.
        size: TerminalSize,
        /// Scrollback capacity which could not be projected.
        scrollback_rows: usize,
    },
}

impl fmt::Display for TerminalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSize(size) => write!(
                formatter,
                "terminal size must be non-zero, got {}x{}",
                size.columns, size.rows
            ),
            Self::RevisionOverflow => write!(formatter, "terminal revision overflow"),
            Self::ResourceProjectionOverflow {
                size,
                scrollback_rows,
            } => write!(
                formatter,
                "terminal resource projection overflow for {}x{} with {scrollback_rows} scrollback rows",
                size.columns, size.rows
            ),
        }
    }
}

impl std::error::Error for TerminalError {}

/// Host-authoritative terminal model.
pub struct TerminalModel {
    parser: vt100::Parser<SafeCallbacks>,
    revision: u64,
    scrollback_rows: usize,
    resource_projection: TerminalResourceProjection,
}

impl TerminalModel {
    /// Creates a terminal with fixed, bounded main-screen scrollback.
    pub fn new(size: TerminalSize, scrollback_rows: usize) -> Result<Self, TerminalError> {
        validate_size(size)?;
        let resource_projection = project_resources(size, scrollback_rows)?;
        Ok(Self {
            parser: vt100::Parser::new_with_callbacks(
                size.rows,
                size.columns,
                scrollback_rows,
                SafeCallbacks::default(),
            ),
            revision: 0,
            scrollback_rows,
            resource_projection,
        })
    }

    /// Returns the current monotonically increasing revision.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Returns the current viewport size.
    #[must_use]
    pub fn size(&self) -> TerminalSize {
        let (rows, columns) = self.parser.screen().size();
        TerminalSize::new(rows, columns)
    }

    /// Processes one ordered PTY byte chunk.
    ///
    /// Empty chunks are no-ops. Every non-empty chunk advances the revision
    /// exactly once after first proving that the revision cannot overflow.
    pub fn ingest(&mut self, bytes: &[u8]) -> Result<TerminalUpdate, TerminalError> {
        if bytes.is_empty() {
            return Ok(TerminalUpdate {
                revision: self.revision,
                replies: Vec::new(),
                events: Vec::new(),
            });
        }

        let next_revision = self.next_revision()?;
        self.parser.process(bytes);
        self.revision = next_revision;
        let (replies, events) = self.parser.callbacks_mut().take_output();
        Ok(TerminalUpdate {
            revision: self.revision,
            replies,
            events,
        })
    }

    /// Resizes the viewport and advances the revision exactly once.
    pub fn resize(&mut self, size: TerminalSize) -> Result<TerminalUpdate, TerminalError> {
        validate_size(size)?;
        let next_revision = self.next_revision()?;
        let resource_projection = project_resources(size, self.scrollback_rows)?;
        self.parser.screen_mut().set_size(size.rows, size.columns);
        self.revision = next_revision;
        self.resource_projection = resource_projection;
        Ok(TerminalUpdate {
            revision: self.revision,
            replies: Vec::new(),
            events: Vec::new(),
        })
    }

    /// Captures an opaque baseline for a later merged delta.
    #[must_use]
    pub fn checkpoint(&self) -> TerminalCheckpoint {
        TerminalCheckpoint {
            revision: self.revision,
            size: self.size(),
            active_screen: active_screen(self.parser.screen()),
            focus_reporting: self.parser.callbacks().focus_reporting,
            screen: self.parser.screen().clone(),
        }
    }

    /// Captures a full reconnect snapshot of the latest state.
    #[must_use]
    pub fn snapshot(&self) -> TerminalSnapshot {
        let screen = self.parser.screen();
        let active_screen = active_screen(screen);
        let modes = terminal_modes(screen, self.parser.callbacks().focus_reporting);
        let mut screen_ansi = Vec::new();
        screen_ansi.extend_from_slice(MAIN_SCREEN);
        if active_screen == ActiveScreen::Alternate {
            screen_ansi.extend_from_slice(ALTERNATE_SCREEN);
        }
        screen_ansi.extend_from_slice(&screen.state_formatted());
        screen_ansi.extend_from_slice(focus_reporting_ansi(modes.focus_reporting));

        TerminalSnapshot {
            revision: self.revision,
            size: self.size(),
            active_screen,
            screen_ansi,
            recent_history_ansi: recent_history_ansi(screen),
            modes,
        }
    }

    /// Produces one merged latest-state delta or a full resynchronization.
    #[must_use]
    pub fn delta_or_resync(&self, checkpoint: &TerminalCheckpoint) -> TerminalDeltaResult {
        let snapshot = self.snapshot();
        if checkpoint.revision > self.revision || checkpoint.size != snapshot.size {
            return TerminalDeltaResult::Resync(snapshot);
        }

        let screen = self.parser.screen();
        let mut ansi = Vec::new();
        if checkpoint.active_screen == snapshot.active_screen {
            ansi.extend_from_slice(&screen.state_diff(&checkpoint.screen));
        } else {
            ansi.extend_from_slice(screen_selection_ansi(snapshot.active_screen));
            ansi.extend_from_slice(&screen.state_formatted());
        }
        if checkpoint.focus_reporting != snapshot.modes.focus_reporting {
            ansi.extend_from_slice(focus_reporting_ansi(snapshot.modes.focus_reporting));
        }

        let delta = TerminalDelta {
            from_revision: checkpoint.revision,
            to_revision: self.revision,
            size: snapshot.size,
            active_screen: snapshot.active_screen,
            ansi,
            modes: snapshot.modes,
        };
        if delta.ansi_payload_len() >= snapshot.ansi_payload_len() {
            TerminalDeltaResult::Resync(snapshot)
        } else {
            TerminalDeltaResult::Delta(delta)
        }
    }

    /// Returns a zterm-owned semantic projection of the visible state.
    #[must_use]
    pub fn state(&self) -> TerminalState {
        let screen = self.parser.screen();
        let size = self.size();
        let mut cells =
            Vec::with_capacity(usize::from(size.rows).saturating_mul(usize::from(size.columns)));
        for row in 0..size.rows {
            for column in 0..size.columns {
                cells.push(
                    screen
                        .cell(row, column)
                        .map_or_else(TerminalCell::default, terminal_cell),
                );
            }
        }
        let (row, column) = screen.cursor_position();
        TerminalState {
            size,
            active_screen: active_screen(screen),
            cursor: TerminalCursor {
                row,
                column,
                visible: !screen.hide_cursor(),
                style: active_style(screen),
            },
            modes: terminal_modes(screen, self.parser.callbacks().focus_reporting),
            cells,
        }
    }

    /// Returns the structural fixed-cell projection for this model.
    #[must_use]
    pub const fn resource_projection(&self) -> TerminalResourceProjection {
        self.resource_projection
    }

    fn next_revision(&self) -> Result<u64, TerminalError> {
        self.revision
            .checked_add(1)
            .ok_or(TerminalError::RevisionOverflow)
    }
}

#[derive(Default)]
struct SafeCallbacks {
    replies: Vec<u8>,
    events: Vec<TerminalSideEvent>,
    dropped_events: u64,
    focus_reporting: bool,
}

impl SafeCallbacks {
    fn push_event(&mut self, event: TerminalSideEvent) {
        if self.events.len() < MAX_SIDE_EVENTS_PER_UPDATE {
            self.events.push(event);
        } else {
            self.dropped_events = self.dropped_events.saturating_add(1);
        }
    }

    fn take_output(&mut self) -> (Vec<u8>, Vec<TerminalSideEvent>) {
        let replies = mem::take(&mut self.replies);
        let mut events = mem::take(&mut self.events);
        if self.dropped_events > 0 {
            if events.len() == MAX_SIDE_EVENTS_PER_UPDATE {
                events.pop();
                self.dropped_events = self.dropped_events.saturating_add(1);
            }
            events.push(TerminalSideEvent::EventsDropped {
                count: self.dropped_events,
            });
            self.dropped_events = 0;
        }
        (replies, events)
    }

    fn push_cursor_report(&mut self, screen: &vt100::Screen, private: bool) {
        let (row, column) = screen.cursor_position();
        let (rows, columns) = screen.size();
        let private_marker = if private { "?" } else { "" };
        self.replies.extend_from_slice(
            format!(
                "\x1b[{private_marker}{};{}R",
                row.min(rows.saturating_sub(1)).saturating_add(1),
                column.min(columns.saturating_sub(1)).saturating_add(1)
            )
            .as_bytes(),
        );
    }
}

impl vt100::Callbacks for SafeCallbacks {
    fn audible_bell(&mut self, _: &mut vt100::Screen) {
        self.push_event(TerminalSideEvent::AudibleBell);
    }

    fn visual_bell(&mut self, _: &mut vt100::Screen) {
        self.push_event(TerminalSideEvent::VisualBell);
    }

    fn resize(&mut self, _: &mut vt100::Screen, request: (u16, u16)) {
        self.push_event(TerminalSideEvent::ResizeRequested(TerminalSize::new(
            request.0, request.1,
        )));
    }

    fn set_window_icon_name(&mut self, _: &mut vt100::Screen, icon_name: &[u8]) {
        let (icon_name, truncated) = bounded_text(icon_name);
        self.push_event(TerminalSideEvent::IconNameChanged {
            icon_name,
            truncated,
        });
    }

    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        let (title, truncated) = bounded_text(title);
        self.push_event(TerminalSideEvent::TitleChanged { title, truncated });
    }

    fn copy_to_clipboard(&mut self, _: &mut vt100::Screen, _: &[u8], _: &[u8]) {
        self.push_event(TerminalSideEvent::EffectRejected(
            RejectedEffect::ClipboardWrite,
        ));
    }

    fn paste_from_clipboard(&mut self, _: &mut vt100::Screen, _: &[u8]) {
        self.push_event(TerminalSideEvent::EffectRejected(
            RejectedEffect::ClipboardRead,
        ));
    }

    fn unhandled_char(&mut self, _: &mut vt100::Screen, _: char) {
        self.push_event(TerminalSideEvent::UnsupportedSequence(
            UnsupportedSequenceKind::Character,
        ));
    }

    fn unhandled_control(&mut self, _: &mut vt100::Screen, _: u8) {
        self.push_event(TerminalSideEvent::UnsupportedSequence(
            UnsupportedSequenceKind::Control,
        ));
    }

    fn unhandled_escape(&mut self, _: &mut vt100::Screen, _: Option<u8>, _: Option<u8>, _: u8) {
        self.push_event(TerminalSideEvent::UnsupportedSequence(
            UnsupportedSequenceKind::Escape,
        ));
    }

    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        intermediate_1: Option<u8>,
        intermediate_2: Option<u8>,
        params: &[&[u16]],
        command: char,
    ) {
        if intermediate_1 == Some(b'?')
            && intermediate_2.is_none()
            && matches!(command, 'h' | 'l')
            && params.iter().any(|param| **param == [1004])
        {
            self.focus_reporting = command == 'h';
            return;
        }

        if intermediate_1.is_none()
            && intermediate_2.is_none()
            && command == 'c'
            && empty_or_single_param(params, 0)
        {
            self.replies
                .extend_from_slice(PRIMARY_DEVICE_ATTRIBUTES_REPLY);
            return;
        }

        if intermediate_2.is_none() && command == 'n' {
            if intermediate_1.is_none() && single_param(params, 5) {
                self.replies.extend_from_slice(DEVICE_STATUS_OK_REPLY);
                return;
            }
            if matches!(intermediate_1, None | Some(b'?')) && single_param(params, 6) {
                self.push_cursor_report(screen, intermediate_1 == Some(b'?'));
                return;
            }
        }

        self.push_event(TerminalSideEvent::UnsupportedSequence(
            UnsupportedSequenceKind::Csi,
        ));
    }

    fn unhandled_osc(&mut self, _: &mut vt100::Screen, _: &[&[u8]]) {
        self.push_event(TerminalSideEvent::UnsupportedSequence(
            UnsupportedSequenceKind::Osc,
        ));
    }
}

fn validate_size(size: TerminalSize) -> Result<(), TerminalError> {
    if size.rows == 0 || size.columns == 0 {
        Err(TerminalError::InvalidSize(size))
    } else {
        Ok(())
    }
}

fn project_resources(
    size: TerminalSize,
    scrollback_rows: usize,
) -> Result<TerminalResourceProjection, TerminalError> {
    let overflow = || TerminalError::ResourceProjectionOverflow {
        size,
        scrollback_rows,
    };
    let visible_cells_per_screen = usize::from(size.rows)
        .checked_mul(usize::from(size.columns))
        .ok_or_else(overflow)?;
    let scrollback_capacity_cells = scrollback_rows
        .checked_mul(usize::from(size.columns))
        .ok_or_else(overflow)?;
    let total_cell_capacity = visible_cells_per_screen
        .checked_mul(2)
        .and_then(|visible| visible.checked_add(scrollback_capacity_cells))
        .ok_or_else(overflow)?;
    let estimated_cell_storage_bytes = total_cell_capacity
        .checked_mul(mem::size_of::<vt100::Cell>())
        .ok_or_else(overflow)?;
    Ok(TerminalResourceProjection {
        visible_cells_per_screen,
        scrollback_capacity_cells,
        total_cell_capacity,
        estimated_cell_storage_bytes,
    })
}

fn active_screen(screen: &vt100::Screen) -> ActiveScreen {
    if screen.alternate_screen() {
        ActiveScreen::Alternate
    } else {
        ActiveScreen::Main
    }
}

fn terminal_modes(screen: &vt100::Screen, focus_reporting: bool) -> TerminalModes {
    TerminalModes {
        application_keypad: screen.application_keypad(),
        application_cursor: screen.application_cursor(),
        bracketed_paste: screen.bracketed_paste(),
        focus_reporting,
        mouse_mode: match screen.mouse_protocol_mode() {
            vt100::MouseProtocolMode::None => TerminalMouseMode::None,
            vt100::MouseProtocolMode::Press => TerminalMouseMode::Press,
            vt100::MouseProtocolMode::PressRelease => TerminalMouseMode::PressRelease,
            vt100::MouseProtocolMode::ButtonMotion => TerminalMouseMode::ButtonMotion,
            vt100::MouseProtocolMode::AnyMotion => TerminalMouseMode::AnyMotion,
        },
        mouse_encoding: match screen.mouse_protocol_encoding() {
            vt100::MouseProtocolEncoding::Default => TerminalMouseEncoding::Default,
            vt100::MouseProtocolEncoding::Utf8 => TerminalMouseEncoding::Utf8,
            vt100::MouseProtocolEncoding::Sgr => TerminalMouseEncoding::Sgr,
        },
    }
}

fn terminal_cell(cell: &vt100::Cell) -> TerminalCell {
    TerminalCell {
        contents: cell.contents().to_owned(),
        wide: cell.is_wide(),
        wide_continuation: cell.is_wide_continuation(),
        style: TerminalStyle {
            foreground: terminal_color(cell.fgcolor()),
            background: terminal_color(cell.bgcolor()),
            bold: cell.bold(),
            dim: cell.dim(),
            italic: cell.italic(),
            underline: cell.underline(),
            inverse: cell.inverse(),
        },
    }
}

fn active_style(screen: &vt100::Screen) -> TerminalStyle {
    TerminalStyle {
        foreground: terminal_color(screen.fgcolor()),
        background: terminal_color(screen.bgcolor()),
        bold: screen.bold(),
        dim: screen.dim(),
        italic: screen.italic(),
        underline: screen.underline(),
        inverse: screen.inverse(),
    }
}

const fn terminal_color(color: vt100::Color) -> TerminalColor {
    match color {
        vt100::Color::Default => TerminalColor::Default,
        vt100::Color::Idx(index) => TerminalColor::Indexed(index),
        vt100::Color::Rgb(red, green, blue) => TerminalColor::Rgb(red, green, blue),
    }
}

fn bounded_text(bytes: &[u8]) -> (String, bool) {
    let truncated = bytes.len() > MAX_TITLE_BYTES;
    let retained = bytes.get(..MAX_TITLE_BYTES).unwrap_or(bytes);
    (String::from_utf8_lossy(retained).into_owned(), truncated)
}

fn empty_or_single_param(params: &[&[u16]], expected: u16) -> bool {
    params.is_empty() || single_param(params, expected)
}

fn single_param(params: &[&[u16]], expected: u16) -> bool {
    params.len() == 1 && params[0] == [expected]
}

const fn screen_selection_ansi(active_screen: ActiveScreen) -> &'static [u8] {
    match active_screen {
        ActiveScreen::Main => MAIN_SCREEN,
        ActiveScreen::Alternate => ALTERNATE_SCREEN,
    }
}

const fn focus_reporting_ansi(enabled: bool) -> &'static [u8] {
    if enabled {
        FOCUS_REPORTING_ON
    } else {
        FOCUS_REPORTING_OFF
    }
}

fn recent_history_ansi(screen: &vt100::Screen) -> Vec<u8> {
    let mut history_screen = screen.clone();
    history_screen.set_scrollback(usize::MAX);
    let history_rows = history_screen.scrollback();
    if history_rows == 0 {
        return Vec::new();
    }

    let (rows, columns) = history_screen.size();
    let page_rows = usize::from(rows);
    let mut output = b"\x1b[m".to_vec();
    let mut emitted = 0;
    while emitted < history_rows {
        let remaining = history_rows - emitted;
        history_screen.set_scrollback(remaining);
        let take = remaining.min(page_rows);
        for line in history_screen.rows(0, columns).take(take) {
            output.extend_from_slice(line.as_bytes());
            output.extend_from_slice(b"\r\n");
        }
        emitted += take;
    }
    for _ in 1..rows {
        output.extend_from_slice(b"\r\n");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revision_overflow_never_mutates_terminal_state() {
        let mut model =
            TerminalModel::new(TerminalSize::new(2, 8), 0).expect("small terminal model is valid");
        model.revision = u64::MAX;
        let before = model.state();

        assert_eq!(
            model.ingest(b"not rendered"),
            Err(TerminalError::RevisionOverflow)
        );
        assert_eq!(model.state(), before);
        assert_eq!(
            model.resize(TerminalSize::new(3, 9)),
            Err(TerminalError::RevisionOverflow)
        );
        assert_eq!(model.state(), before);
    }
}
