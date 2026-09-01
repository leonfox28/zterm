//! Host-authoritative terminal state and reconnect snapshots.
//!
//! The current implementation uses `vt100` internally, but every public type
//! is owned by zterm. The Foundation Gate reports VT100 primary device
//! attributes with the Advanced Video Option (`CSI ? 1;2 c`); it does not yet
//! assign a richer `TERM` value such as `xterm-256color`.

use std::{fmt, mem};

use crate::Revision;

const PRIMARY_DEVICE_ATTRIBUTES_REPLY: &[u8] = b"\x1b[?1;2c";
const DEVICE_STATUS_OK_REPLY: &[u8] = b"\x1b[0n";
/// Exact selector prefixed to daemon-authored ANSI for the main screen.
///
/// A renderer which already owns its outer alternate screen may consume this
/// prefix as zterm metadata instead of forwarding it to the physical terminal.
pub const MAIN_SCREEN_SELECTION_ANSI: &[u8] = b"\x1b[?1049l";

/// Exact selector prefixed to daemon-authored ANSI for the alternate screen.
///
/// This is paired with [`MAIN_SCREEN_SELECTION_ANSI`] and is not a general
/// terminal-parser surface.
pub const ALTERNATE_SCREEN_SELECTION_ANSI: &[u8] = b"\x1b[?1049h";
const FOCUS_REPORTING_ON: &[u8] = b"\x1b[?1004h";
const FOCUS_REPORTING_OFF: &[u8] = b"\x1b[?1004l";
const ALTERNATE_SCROLL_ON: &[u8] = b"\x1b[?1007h";
const ALTERNATE_SCROLL_OFF: &[u8] = b"\x1b[?1007l";

/// Maximum number of side events returned by one terminal update.
pub const MAX_SIDE_EVENTS_PER_UPDATE: usize = 32;

/// Maximum number of source bytes retained for a title or icon-name event.
pub const MAX_TITLE_BYTES: usize = 256;

/// Maximum number of daemon-authored rows returned by one history page.
pub const MAX_HISTORY_PAGE_ROWS: usize = 80;

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
#[derive(Clone, Default, Eq, PartialEq)]
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

impl fmt::Debug for TerminalCell {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalCell")
            .field("contents", &"[REDACTED]")
            .field("contents_len", &self.contents.len())
            .field("wide", &self.wide)
            .field("wide_continuation", &self.wide_continuation)
            .field("style", &self.style)
            .finish()
    }
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
    /// Wheel input should be translated to cursor keys on the alternate screen.
    pub alternate_scroll: bool,
    /// Mouse event reporting mode.
    pub mouse_mode: TerminalMouseMode,
    /// Mouse event encoding.
    pub mouse_encoding: TerminalMouseEncoding,
}

/// Direction of one bounded history request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalHistoryDirection {
    /// Return the newest retained history rows.
    Newest,
    /// Return rows immediately older than the supplied cursor.
    Older,
    /// Return rows immediately newer than the supplied cursor.
    Newer,
}

/// Stable position of one daemon-authored history page.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalHistoryCursor {
    /// Epoch which changes whenever retained row identity may have changed.
    pub epoch: Revision,
    /// Model revision observed while producing this page.
    pub revision: Revision,
    /// Zero-based page start measured from the oldest retained history row.
    pub start_row: u64,
    /// Number of rows in this page.
    pub row_count: u32,
    /// Oldest retained row bound, currently always zero.
    pub oldest_row: u64,
    /// Exclusive newest retained row bound.
    pub newest_row: u64,
}

/// One bounded daemon-formatted history page.
#[derive(Clone, Eq, PartialEq)]
pub struct TerminalHistoryPage {
    /// Stable cursor and retained bounds for this page.
    pub cursor: TerminalHistoryCursor,
    /// Independently formatted ANSI rows in oldest-to-newest order.
    pub rows: Vec<Vec<u8>>,
}

impl fmt::Debug for TerminalHistoryPage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalHistoryPage")
            .field("cursor", &self.cursor)
            .field("row_count", &self.rows.len())
            .field("ansi_bytes", &self.rows.iter().map(Vec::len).sum::<usize>())
            .finish()
    }
}

/// Result of resolving a revision-bound history request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerminalHistoryResult {
    /// The requested rows were produced from one consistent retained epoch.
    Page(TerminalHistoryPage),
    /// The terminal or retained-history epoch changed; callers must start over.
    HistoryChanged {
        /// Current history epoch.
        epoch: Revision,
        /// Current model revision.
        revision: Revision,
    },
    /// The supplied cursor or range no longer names retained rows.
    HistoryGap {
        /// Current history epoch.
        epoch: Revision,
        /// Current model revision.
        revision: Revision,
    },
}

/// Zterm-owned semantic projection used to compare terminal states.
#[derive(Clone, Eq, PartialEq)]
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

impl fmt::Debug for TerminalState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalState")
            .field("size", &self.size)
            .field("active_screen", &self.active_screen)
            .field("cursor", &self.cursor)
            .field("modes", &self.modes)
            .field("cell_count", &self.cells.len())
            .finish()
    }
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
#[derive(Clone, Eq, PartialEq)]
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

impl fmt::Debug for TerminalSideEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AudibleBell => formatter.write_str("AudibleBell"),
            Self::VisualBell => formatter.write_str("VisualBell"),
            Self::ResizeRequested(size) => formatter
                .debug_tuple("ResizeRequested")
                .field(size)
                .finish(),
            Self::TitleChanged { title, truncated } => formatter
                .debug_struct("TitleChanged")
                .field("title", &"[REDACTED]")
                .field("title_len", &title.len())
                .field("truncated", truncated)
                .finish(),
            Self::IconNameChanged {
                icon_name,
                truncated,
            } => formatter
                .debug_struct("IconNameChanged")
                .field("icon_name", &"[REDACTED]")
                .field("icon_name_len", &icon_name.len())
                .field("truncated", truncated)
                .finish(),
            Self::EffectRejected(effect) => formatter
                .debug_tuple("EffectRejected")
                .field(effect)
                .finish(),
            Self::UnsupportedSequence(kind) => formatter
                .debug_tuple("UnsupportedSequence")
                .field(kind)
                .finish(),
            Self::EventsDropped { count } => formatter
                .debug_struct("EventsDropped")
                .field("count", count)
                .finish(),
        }
    }
}

/// Output produced by one ordered ingest or resize operation.
#[derive(Clone, Eq, PartialEq)]
pub struct TerminalUpdate {
    /// Model revision after the operation.
    pub revision: Revision,
    /// Controlled bytes which must be written back to the hosted PTY.
    pub replies: Vec<u8>,
    /// Bounded non-rendering side events.
    pub events: Vec<TerminalSideEvent>,
}

impl fmt::Debug for TerminalUpdate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalUpdate")
            .field("revision", &self.revision)
            .field("replies", &"[REDACTED]")
            .field("reply_len", &self.replies.len())
            .field("events", &self.events)
            .finish()
    }
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
#[derive(Clone, Eq, PartialEq)]
pub struct TerminalSnapshot {
    /// Revision represented by the snapshot.
    pub revision: Revision,
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

impl fmt::Debug for TerminalSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalSnapshot")
            .field("revision", &self.revision)
            .field("size", &self.size)
            .field("active_screen", &self.active_screen)
            .field("screen_ansi", &"[REDACTED]")
            .field("screen_ansi_len", &self.screen_ansi.len())
            .field("recent_history_ansi", &"[REDACTED]")
            .field("recent_history_ansi_len", &self.recent_history_ansi.len())
            .field("modes", &self.modes)
            .finish()
    }
}

impl TerminalSnapshot {
    /// Returns the number of ANSI payload bytes in this snapshot.
    #[must_use]
    pub fn ansi_payload_len(&self) -> usize {
        self.screen_ansi.len() + self.recent_history_ansi.len()
    }

    /// Drops only oldest complete history lines until the ANSI payload fits.
    ///
    /// The current screen is always preserved. The returned value is `false`
    /// only when the screen by itself exceeds `maximum_bytes`.
    pub fn limit_ansi_payload(&mut self, maximum_bytes: usize) -> bool {
        if self.ansi_payload_len() <= maximum_bytes {
            return true;
        }
        let Some(history_budget) = maximum_bytes.checked_sub(self.screen_ansi.len()) else {
            self.recent_history_ansi.clear();
            return false;
        };
        limit_recent_history(&mut self.recent_history_ansi, history_budget);
        self.ansi_payload_len() <= maximum_bytes
    }
}

/// A merged current-screen delta from one checkpoint to the latest revision.
#[derive(Clone, Eq, PartialEq)]
pub struct TerminalDelta {
    /// Checkpoint revision used as the baseline.
    pub from_revision: Revision,
    /// Latest model revision represented by the delta.
    pub to_revision: Revision,
    /// Viewport size represented by the delta.
    pub size: TerminalSize,
    /// Screen selected after applying the delta.
    pub active_screen: ActiveScreen,
    /// ANSI bytes which update the current visible terminal state.
    pub ansi: Vec<u8>,
    /// Input modes represented after applying the delta.
    pub modes: TerminalModes,
}

impl fmt::Debug for TerminalDelta {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalDelta")
            .field("from_revision", &self.from_revision)
            .field("to_revision", &self.to_revision)
            .field("size", &self.size)
            .field("active_screen", &self.active_screen)
            .field("ansi", &"[REDACTED]")
            .field("ansi_len", &self.ansi.len())
            .field("modes", &self.modes)
            .finish()
    }
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
    revision: Revision,
    size: TerminalSize,
    active_screen: ActiveScreen,
    focus_reporting: bool,
    alternate_scroll: bool,
    screen: vt100::Screen,
    retained_cell_capacity: usize,
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

impl TerminalCheckpoint {
    /// Returns the exact authoritative revision represented by this baseline.
    #[doc(hidden)]
    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    /// Returns the fixed visible-cell capacity retained by this checkpoint.
    ///
    /// A checkpoint keeps only main and alternate visible grids. Host
    /// scrollback remains owned once by `TerminalModel` and is never cloned
    /// into an attachment baseline.
    #[doc(hidden)]
    #[must_use]
    pub const fn retained_cell_capacity(&self) -> usize {
        self.retained_cell_capacity
    }

    /// Returns the number of main-screen history rows retained by this baseline.
    #[doc(hidden)]
    #[must_use]
    pub fn retained_scrollback_rows(&self) -> usize {
        let mut screen = self.screen.clone();
        screen.set_scrollback(usize::MAX);
        screen.scrollback()
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
    /// A history page requested zero rows or exceeded the fixed page bound.
    InvalidHistoryPageSize {
        /// Requested number of rows.
        requested: usize,
        /// Product maximum for one page.
        maximum: usize,
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
            Self::InvalidHistoryPageSize { requested, maximum } => write!(
                formatter,
                "terminal history page size {requested} is outside 1..={maximum}",
            ),
        }
    }
}

impl std::error::Error for TerminalError {}

/// Host-authoritative terminal model.
pub struct TerminalModel {
    parser: vt100::Parser<SafeCallbacks>,
    revision: Revision,
    scrollback_rows: usize,
    resource_projection: TerminalResourceProjection,
    history_epoch: Revision,
    retained_history_rows: usize,
}

impl TerminalModel {
    /// Computes the fixed-cell projection without allocating a parser.
    pub fn project_resources(
        size: TerminalSize,
        scrollback_rows: usize,
    ) -> Result<TerminalResourceProjection, TerminalError> {
        validate_size(size)?;
        project_resources(size, scrollback_rows)
    }

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
            revision: Revision::ZERO,
            scrollback_rows,
            resource_projection,
            history_epoch: Revision::ZERO,
            retained_history_rows: 0,
        })
    }

    /// Returns the current monotonically increasing revision.
    #[must_use]
    pub const fn revision(&self) -> Revision {
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
        self.refresh_history_epoch_after_ingest();
        let (replies, events) = self.parser.callbacks_mut().take_output();
        Ok(TerminalUpdate {
            revision: self.revision,
            replies,
            events,
        })
    }

    /// Resizes the viewport and advances the revision exactly once.
    pub fn resize(&mut self, size: TerminalSize) -> Result<TerminalUpdate, TerminalError> {
        let (next_revision, resource_projection) = self.preflight_resize(size)?;
        self.parser.screen_mut().set_size(size.rows, size.columns);
        self.revision = next_revision;
        self.resource_projection = resource_projection;
        self.history_epoch = next_revision;
        self.retained_history_rows = if active_screen(self.parser.screen()) == ActiveScreen::Main {
            retained_history_rows(self.parser.screen())
        } else {
            self.retained_history_rows
        };
        Ok(TerminalUpdate {
            revision: self.revision,
            replies: Vec::new(),
            events: Vec::new(),
        })
    }

    /// Checks a resize without changing parser state or allocating a new model.
    pub fn preflight_resize(
        &self,
        size: TerminalSize,
    ) -> Result<(Revision, TerminalResourceProjection), TerminalError> {
        validate_size(size)?;
        let next_revision = self.next_revision()?;
        let resource_projection = project_resources(size, self.scrollback_rows)?;
        Ok((next_revision, resource_projection))
    }

    /// Captures an opaque baseline for a later merged delta.
    #[must_use]
    pub fn checkpoint(&self) -> TerminalCheckpoint {
        let screen = self.parser.screen();
        let size = self.size();
        let active_screen = active_screen(screen);
        let focus_reporting = self.parser.callbacks().focus_reporting;
        let alternate_scroll = self.parser.callbacks().alternate_scroll;
        let mut baseline =
            vt100::Parser::new_with_callbacks(size.rows, size.columns, 0, SafeCallbacks::default());
        baseline.process(&visible_screen_ansi(
            screen,
            active_screen,
            focus_reporting,
            alternate_scroll,
        ));
        let retained_cell_capacity = usize::from(size.rows)
            .saturating_mul(usize::from(size.columns))
            .saturating_mul(2);
        TerminalCheckpoint {
            revision: self.revision,
            size,
            active_screen,
            focus_reporting,
            alternate_scroll,
            screen: baseline.screen().clone(),
            retained_cell_capacity,
        }
    }

    /// Captures a full reconnect snapshot of the latest state.
    #[must_use]
    pub fn snapshot(&self) -> TerminalSnapshot {
        let screen = self.parser.screen();
        let active_screen = active_screen(screen);
        let callbacks = self.parser.callbacks();
        let modes = terminal_modes(
            screen,
            callbacks.focus_reporting,
            callbacks.alternate_scroll,
        );
        let screen_ansi = visible_screen_ansi(
            screen,
            active_screen,
            modes.focus_reporting,
            modes.alternate_scroll,
        );

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
        if checkpoint.alternate_scroll != snapshot.modes.alternate_scroll {
            ansi.extend_from_slice(alternate_scroll_ansi(snapshot.modes.alternate_scroll));
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
            modes: terminal_modes(
                screen,
                self.parser.callbacks().focus_reporting,
                self.parser.callbacks().alternate_scroll,
            ),
            cells,
        }
    }

    /// Returns the structural fixed-cell projection for this model.
    #[must_use]
    pub const fn resource_projection(&self) -> TerminalResourceProjection {
        self.resource_projection
    }

    /// Returns one bounded, revision-aware page from retained main-screen history.
    pub fn history_page(
        &self,
        direction: TerminalHistoryDirection,
        cursor: Option<TerminalHistoryCursor>,
        maximum_rows: usize,
    ) -> Result<TerminalHistoryResult, TerminalError> {
        if maximum_rows == 0 || maximum_rows > MAX_HISTORY_PAGE_ROWS {
            return Err(TerminalError::InvalidHistoryPageSize {
                requested: maximum_rows,
                maximum: MAX_HISTORY_PAGE_ROWS,
            });
        }
        if active_screen(self.parser.screen()) != ActiveScreen::Main {
            return Ok(TerminalHistoryResult::HistoryChanged {
                epoch: self.history_epoch,
                revision: self.revision,
            });
        }

        let total = self.retained_history_rows;
        let total_u64 = u64::try_from(total).unwrap_or(u64::MAX);
        let start = match direction {
            TerminalHistoryDirection::Newest => total.saturating_sub(maximum_rows),
            TerminalHistoryDirection::Older | TerminalHistoryDirection::Newer => {
                let Some(cursor) = cursor else {
                    return Ok(TerminalHistoryResult::HistoryGap {
                        epoch: self.history_epoch,
                        revision: self.revision,
                    });
                };
                if cursor.epoch != self.history_epoch {
                    return Ok(TerminalHistoryResult::HistoryChanged {
                        epoch: self.history_epoch,
                        revision: self.revision,
                    });
                }
                let end = cursor.start_row.checked_add(u64::from(cursor.row_count));
                if cursor.oldest_row != 0
                    || cursor.start_row < cursor.oldest_row
                    || end.is_none_or(|end| end > cursor.newest_row)
                    || cursor.newest_row > total_u64
                {
                    return Ok(TerminalHistoryResult::HistoryGap {
                        epoch: self.history_epoch,
                        revision: self.revision,
                    });
                }
                match direction {
                    TerminalHistoryDirection::Older => usize::try_from(cursor.start_row)
                        .unwrap_or(usize::MAX)
                        .saturating_sub(maximum_rows),
                    TerminalHistoryDirection::Newer => {
                        usize::try_from(end.unwrap_or(total_u64)).unwrap_or(usize::MAX)
                    }
                    TerminalHistoryDirection::Newest => unreachable!(),
                }
            }
        };
        if start > total {
            return Ok(TerminalHistoryResult::HistoryGap {
                epoch: self.history_epoch,
                revision: self.revision,
            });
        }
        let count = maximum_rows.min(total - start);
        let rows = formatted_history_rows(self.parser.screen(), total, start, count);
        let row_count = u32::try_from(rows.len()).unwrap_or(u32::MAX);
        Ok(TerminalHistoryResult::Page(TerminalHistoryPage {
            cursor: TerminalHistoryCursor {
                epoch: self.history_epoch,
                revision: self.revision,
                start_row: u64::try_from(start).unwrap_or(u64::MAX),
                row_count,
                oldest_row: 0,
                newest_row: total_u64,
            },
            rows,
        }))
    }

    fn next_revision(&self) -> Result<Revision, TerminalError> {
        self.revision
            .checked_next()
            .ok_or(TerminalError::RevisionOverflow)
    }

    fn refresh_history_epoch_after_ingest(&mut self) {
        if active_screen(self.parser.screen()) != ActiveScreen::Main {
            return;
        }
        let retained = retained_history_rows(self.parser.screen());
        let reached_capacity = self.scrollback_rows > 0
            && retained == self.scrollback_rows
            && self.retained_history_rows <= retained;
        if retained < self.retained_history_rows || reached_capacity {
            self.history_epoch = self.revision;
        }
        self.retained_history_rows = retained;
    }
}

#[derive(Default)]
struct SafeCallbacks {
    replies: Vec<u8>,
    events: Vec<TerminalSideEvent>,
    dropped_events: u64,
    focus_reporting: bool,
    alternate_scroll: bool,
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
            && params
                .iter()
                .any(|param| matches!(**param, [1004] | [1007]))
        {
            for param in params {
                match **param {
                    [1004] => self.focus_reporting = command == 'h',
                    [1007] => self.alternate_scroll = command == 'h',
                    _ => {}
                }
            }
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

fn terminal_modes(
    screen: &vt100::Screen,
    focus_reporting: bool,
    alternate_scroll: bool,
) -> TerminalModes {
    TerminalModes {
        application_keypad: screen.application_keypad(),
        application_cursor: screen.application_cursor(),
        bracketed_paste: screen.bracketed_paste(),
        focus_reporting,
        alternate_scroll,
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

fn visible_screen_ansi(
    screen: &vt100::Screen,
    active_screen: ActiveScreen,
    focus_reporting: bool,
    alternate_scroll: bool,
) -> Vec<u8> {
    let mut ansi = Vec::new();
    ansi.extend_from_slice(MAIN_SCREEN_SELECTION_ANSI);
    if active_screen == ActiveScreen::Alternate {
        ansi.extend_from_slice(ALTERNATE_SCREEN_SELECTION_ANSI);
    }
    ansi.extend_from_slice(&screen.state_formatted());
    ansi.extend_from_slice(focus_reporting_ansi(focus_reporting));
    ansi.extend_from_slice(alternate_scroll_ansi(alternate_scroll));
    ansi
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
        ActiveScreen::Main => MAIN_SCREEN_SELECTION_ANSI,
        ActiveScreen::Alternate => ALTERNATE_SCREEN_SELECTION_ANSI,
    }
}

const fn focus_reporting_ansi(enabled: bool) -> &'static [u8] {
    if enabled {
        FOCUS_REPORTING_ON
    } else {
        FOCUS_REPORTING_OFF
    }
}

const fn alternate_scroll_ansi(enabled: bool) -> &'static [u8] {
    if enabled {
        ALTERNATE_SCROLL_ON
    } else {
        ALTERNATE_SCROLL_OFF
    }
}

fn retained_history_rows(screen: &vt100::Screen) -> usize {
    let mut history = screen.clone();
    history.set_scrollback(usize::MAX);
    history.scrollback()
}

fn formatted_history_rows(
    screen: &vt100::Screen,
    total: usize,
    start: usize,
    count: usize,
) -> Vec<Vec<u8>> {
    let mut history = screen.clone();
    let (_, columns) = history.size();
    let mut rows = Vec::with_capacity(count);
    for index in start..start.saturating_add(count) {
        history.set_scrollback(total.saturating_sub(index));
        let mut row = b"\x1b[m".to_vec();
        if let Some(formatted) = history.rows_formatted(0, columns).next() {
            row.extend_from_slice(&formatted);
        }
        row.extend_from_slice(b"\x1b[m");
        rows.push(row);
    }
    rows
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

fn limit_recent_history(history: &mut Vec<u8>, maximum_bytes: usize) {
    const RESET: &[u8] = b"\x1b[m";
    const LINE_END: &[u8] = b"\r\n";

    if history.len() <= maximum_bytes {
        return;
    }
    if maximum_bytes < RESET.len() + LINE_END.len() || !history.starts_with(RESET) {
        history.clear();
        return;
    }

    let suffix_budget = maximum_bytes - RESET.len();
    let desired = history.len().saturating_sub(suffix_budget).max(RESET.len());
    let starts_on_boundary = desired >= LINE_END.len()
        && history.get(desired - LINE_END.len()..desired) == Some(LINE_END);
    let start = if starts_on_boundary {
        desired
    } else {
        let Some(boundary) = history.get(desired..).and_then(|suffix| {
            suffix
                .windows(LINE_END.len())
                .position(|bytes| bytes == LINE_END)
        }) else {
            history.clear();
            return;
        };
        desired + boundary + LINE_END.len()
    };
    if start >= history.len() {
        history.clear();
        return;
    }

    let retained = history.len() - start;
    history.copy_within(start.., RESET.len());
    history[..RESET.len()].copy_from_slice(RESET);
    history.truncate(RESET.len() + retained);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revision_overflow_never_mutates_terminal_state() {
        let mut model =
            TerminalModel::new(TerminalSize::new(2, 8), 0).expect("small terminal model is valid");
        model.revision = Revision::new(u64::MAX);
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

    #[test]
    fn projection_and_resize_preflight_do_not_mutate() {
        let size = TerminalSize::new(40, 120);
        let projected = TerminalModel::project_resources(size, 2_000)
            .expect("foundation projection is representable");
        let model = TerminalModel::new(size, 2_000).expect("foundation model");
        assert_eq!(model.resource_projection(), projected);
        let before = model.state();
        let (revision, resized) = model
            .preflight_resize(TerminalSize::new(80, 240))
            .expect("maximum viewport preflights");
        assert_eq!(revision, Revision::new(1));
        assert!(resized.estimated_cell_storage_bytes > projected.estimated_cell_storage_bytes);
        assert_eq!(model.state(), before);
        assert!(matches!(
            TerminalModel::project_resources(TerminalSize::new(0, 1), 2_000),
            Err(TerminalError::InvalidSize(_))
        ));
    }

    #[test]
    fn snapshot_wire_limit_preserves_screen_and_complete_recent_lines() {
        let screen = b"authoritative-screen".to_vec();
        let mut snapshot = TerminalSnapshot {
            revision: Revision::new(9),
            size: TerminalSize::new(2, 20),
            active_screen: ActiveScreen::Main,
            screen_ansi: screen.clone(),
            recent_history_ansi: b"\x1b[mold-line\r\nrecent-one\r\nrecent-two\r\n".to_vec(),
            modes: TerminalModes::default(),
        };
        let maximum = screen.len() + b"\x1b[mrecent-two\r\n".len();
        assert!(snapshot.limit_ansi_payload(maximum));
        assert_eq!(snapshot.screen_ansi, screen);
        assert_eq!(snapshot.recent_history_ansi, b"\x1b[mrecent-two\r\n");
        assert!(snapshot.ansi_payload_len() <= maximum);

        assert!(!snapshot.limit_ansi_payload(screen.len() - 1));
        assert_eq!(snapshot.screen_ansi, screen);
        assert!(snapshot.recent_history_ansi.is_empty());
    }

    #[test]
    fn history_pages_are_ordered_revision_bound_and_non_mutating() {
        let mut model =
            TerminalModel::new(TerminalSize::new(2, 12), 8).expect("bounded history terminal");
        model
            .ingest(b"one\r\ntwo\r\nthree\r\nfour\r\nfive")
            .expect("seed history");
        let before = model.state();
        let TerminalHistoryResult::Page(page) = model
            .history_page(TerminalHistoryDirection::Newest, None, 2)
            .expect("newest page")
        else {
            panic!("newest history must be available");
        };
        assert_eq!(page.rows.len(), 2);
        assert!(String::from_utf8_lossy(&page.rows[0]).contains("two"));
        assert!(String::from_utf8_lossy(&page.rows[1]).contains("three"));
        assert_eq!(
            model.state(),
            before,
            "paging never changes parser scrollback"
        );

        model.ingest(b"\r\nsix").expect("append below capacity");
        let TerminalHistoryResult::Page(newer) = model
            .history_page(
                TerminalHistoryDirection::Newer,
                Some(page.cursor),
                MAX_HISTORY_PAGE_ROWS,
            )
            .expect("newer page after monotonic append")
        else {
            panic!("monotonic append keeps the history epoch");
        };
        assert_eq!(newer.cursor.epoch, page.cursor.epoch);
        assert!(
            newer
                .rows
                .iter()
                .any(|row| String::from_utf8_lossy(row).contains("four"))
        );

        model
            .resize(TerminalSize::new(3, 12))
            .expect("resize invalidates row identity");
        assert!(matches!(
            model
                .history_page(TerminalHistoryDirection::Older, Some(page.cursor), 2,)
                .expect("typed stale result"),
            TerminalHistoryResult::HistoryChanged { .. }
        ));
    }

    #[test]
    fn history_capacity_and_alternate_screen_fail_conservatively() {
        let mut model = TerminalModel::new(TerminalSize::new(2, 10), 2)
            .expect("small bounded history terminal");
        model.ingest(b"one\r\ntwo\r\nthree").expect("fill history");
        let TerminalHistoryResult::Page(page) = model
            .history_page(TerminalHistoryDirection::Newest, None, 2)
            .expect("initial page")
        else {
            panic!("initial page must exist");
        };
        model
            .ingest(b"\r\nfour\r\nfive")
            .expect("evict old history");
        assert!(matches!(
            model
                .history_page(TerminalHistoryDirection::Older, Some(page.cursor), 2,)
                .expect("typed eviction result"),
            TerminalHistoryResult::HistoryChanged { .. }
        ));

        model
            .ingest(b"\x1b[?1049h")
            .expect("enter alternate screen");
        assert!(matches!(
            model
                .history_page(TerminalHistoryDirection::Newest, None, 2)
                .expect("typed alternate result"),
            TerminalHistoryResult::HistoryChanged { .. }
        ));
    }

    #[test]
    fn alternate_scroll_round_trips_through_snapshots_and_deltas() {
        let mut model =
            TerminalModel::new(TerminalSize::new(2, 10), 2).expect("alternate-scroll model");
        model.ingest(b"\x1b[?1007h").expect("enable mode");
        assert!(model.snapshot().modes.alternate_scroll);
        let checkpoint = model.checkpoint();
        model.ingest(b"\x1b[?1007l").expect("disable mode");
        assert!(!model.state().modes.alternate_scroll);
        match model.delta_or_resync(&checkpoint) {
            TerminalDeltaResult::Delta(delta) => {
                assert!(!delta.modes.alternate_scroll);
                assert!(
                    delta
                        .ansi
                        .windows(ALTERNATE_SCROLL_OFF.len())
                        .any(|bytes| { bytes == ALTERNATE_SCROLL_OFF })
                );
            }
            TerminalDeltaResult::Resync(snapshot) => {
                assert!(!snapshot.modes.alternate_scroll);
            }
        }
    }

    #[test]
    fn terminal_debug_redacts_content_and_retains_structural_metadata() {
        const CELL_SENTINEL: &str = "TERM_CELL_SENTINEL_7f0d";
        const TITLE_SENTINEL: &str = "TERM_TITLE_SENTINEL_154c";
        const ICON_SENTINEL: &str = "TERM_ICON_SENTINEL_861a";
        const REPLY_SENTINEL: &[u8] = b"TERM_REPLY_SENTINEL_23d9";
        const SCREEN_SENTINEL: &[u8] = b"TERM_SCREEN_SENTINEL_3ba7";
        const HISTORY_SENTINEL: &[u8] = b"TERM_HISTORY_SENTINEL_9ca2";
        const DELTA_SENTINEL: &[u8] = b"TERM_DELTA_SENTINEL_c156";

        let cell = TerminalCell {
            contents: CELL_SENTINEL.to_owned(),
            wide: true,
            wide_continuation: false,
            style: TerminalStyle {
                bold: true,
                ..TerminalStyle::default()
            },
        };
        let state = TerminalState {
            size: TerminalSize::new(41, 137),
            active_screen: ActiveScreen::Alternate,
            cursor: TerminalCursor {
                row: 3,
                column: 5,
                visible: true,
                style: TerminalStyle::default(),
            },
            modes: TerminalModes::default(),
            cells: vec![cell.clone()],
        };
        let update = TerminalUpdate {
            revision: Revision::new(43),
            replies: REPLY_SENTINEL.to_vec(),
            events: vec![
                TerminalSideEvent::TitleChanged {
                    title: TITLE_SENTINEL.to_owned(),
                    truncated: false,
                },
                TerminalSideEvent::IconNameChanged {
                    icon_name: ICON_SENTINEL.to_owned(),
                    truncated: true,
                },
            ],
        };
        let snapshot = TerminalSnapshot {
            revision: Revision::new(47),
            size: TerminalSize::new(41, 137),
            active_screen: ActiveScreen::Main,
            screen_ansi: SCREEN_SENTINEL.to_vec(),
            recent_history_ansi: HISTORY_SENTINEL.to_vec(),
            modes: TerminalModes::default(),
        };
        let delta = TerminalDelta {
            from_revision: Revision::new(47),
            to_revision: Revision::new(53),
            size: TerminalSize::new(41, 137),
            active_screen: ActiveScreen::Main,
            ansi: DELTA_SENTINEL.to_vec(),
            modes: TerminalModes::default(),
        };

        let rendered = format!(
            "{cell:?} {state:?} {update:?} {snapshot:?} {delta:?} {:?} {:?}",
            TerminalDeltaResult::Delta(delta.clone()),
            TerminalDeltaResult::Resync(snapshot.clone()),
        );
        for text in [CELL_SENTINEL, TITLE_SENTINEL, ICON_SENTINEL] {
            assert!(!rendered.contains(text));
        }
        for bytes in [
            REPLY_SENTINEL,
            SCREEN_SENTINEL,
            HISTORY_SENTINEL,
            DELTA_SENTINEL,
        ] {
            assert!(!rendered.contains(std::str::from_utf8(bytes).expect("ASCII sentinel")));
            assert!(!rendered.contains(&format!("{bytes:?}")));
        }
        assert!(rendered.contains("[REDACTED]"));
        assert!(rendered.contains("cell_count: 1"));
        assert!(rendered.contains("rows: 41"));
        assert!(rendered.contains("columns: 137"));
        assert!(rendered.contains(&format!("reply_len: {}", REPLY_SENTINEL.len())));
        assert!(rendered.contains(&format!("ansi_len: {}", DELTA_SENTINEL.len())));

        assert_eq!(cell, cell.clone());
        assert_eq!(state, state.clone());
        assert_eq!(update, update.clone());
        assert_eq!(snapshot, snapshot.clone());
        assert_eq!(delta, delta.clone());
    }
}
