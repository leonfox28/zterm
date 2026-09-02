//! Transport-neutral terminal domain values and reconnect payloads.
//!
//! The host-side parser and grid live in `zterm-terminal`. This module is kept
//! free of terminal-engine dependencies so protocol and future client crates
//! can share these values without compiling a host PTY stack.

use std::fmt;

use crate::Revision;

/// Exact selector prefixed to daemon-authored ANSI for the main screen.
pub const MAIN_SCREEN_SELECTION_ANSI: &[u8] = b"\x1b[?1049l";

/// Exact selector prefixed to daemon-authored ANSI for the alternate screen.
pub const ALTERNATE_SCREEN_SELECTION_ANSI: &[u8] = b"\x1b[?1049h";

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
    /// Creates a viewport size. Host models reject zero dimensions.
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

/// Renderer-neutral position and extent of one attachment-local viewport.
///
/// Offset zero is the live bottom. Positive offsets count logical rows toward
/// older retained main-screen history.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalScrollMetrics {
    /// Epoch which changes whenever retained row identity may have changed.
    pub epoch: Revision,
    /// Model revision observed while producing these metrics.
    pub revision: Revision,
    /// Current logical row offset from the live bottom.
    pub offset_from_bottom: u64,
    /// Largest retained logical row offset available in this epoch.
    pub max_offset_from_bottom: u64,
    /// Full model height represented by a viewport frame.
    pub viewport_rows: u16,
}

impl TerminalScrollMetrics {
    /// Returns whether the metrics describe a valid bounded viewport.
    #[must_use]
    pub const fn is_valid(self) -> bool {
        self.viewport_rows > 0
            && self.epoch.get() <= self.revision.get()
            && self.offset_from_bottom <= self.max_offset_from_bottom
    }
}

/// One semantic attachment-local scroll request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalScrollAction {
    /// Move by logical rows; positive is older/up and negative is newer/down.
    ScrollByLines(i32),
    /// Jump to an absolute logical offset from the live bottom.
    ScrollToOffset(u64),
}

/// Whether a complete history viewport preserved its previous row identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalViewportDisposition {
    /// The requested action resolved within the same retained-history identity.
    Exact,
    /// Retained-history identity changed and the closest current frame replaced it.
    Rebased,
}

/// One complete, daemon-authored attachment viewport.
#[derive(Clone, Eq, PartialEq)]
pub struct TerminalViewportFrame {
    /// Whether the frame is exact or replaces an invalidated history epoch.
    pub disposition: TerminalViewportDisposition,
    /// Position and extent represented by the rows.
    pub metrics: TerminalScrollMetrics,
    /// Independently formatted canonical ANSI rows from top to bottom.
    pub rows: Vec<Vec<u8>>,
}

impl fmt::Debug for TerminalViewportFrame {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalViewportFrame")
            .field("disposition", &self.disposition)
            .field("metrics", &self.metrics)
            .field("row_count", &self.rows.len())
            .field("ansi_bytes", &self.rows.iter().map(Vec::len).sum::<usize>())
            .finish()
    }
}

/// Result of applying one semantic scroll action to an attachment baseline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerminalViewportResult {
    /// A complete non-live viewport is available.
    Frame(TerminalViewportFrame),
    /// The action reached the live bottom; callers must use the live sync path.
    Live(TerminalScrollMetrics),
    /// Main-screen row identity changed and no history frame can be asserted.
    HistoryChanged {
        /// Current retained-history epoch.
        epoch: Revision,
        /// Current model revision.
        revision: Revision,
    },
    /// The supplied baseline was structurally invalid or from the future.
    HistoryGap {
        /// Current retained-history epoch.
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
    /// An unsupported control byte or string.
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
    pub screen_ansi: Vec<u8>,
    /// Bounded standard scrollback encoded as an ANSI-compatible text stream.
    pub recent_history_ansi: Vec<u8>,
    /// Input modes represented by the snapshot.
    pub modes: TerminalModes,
    /// Live main-screen scroll extent, absent when it cannot be asserted.
    pub scroll_metrics: Option<TerminalScrollMetrics>,
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
            .field("scroll_metrics", &self.scroll_metrics)
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
    /// Live main-screen scroll extent, absent when it cannot be asserted.
    pub scroll_metrics: Option<TerminalScrollMetrics>,
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
            .field("scroll_metrics", &self.scroll_metrics)
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
    fn snapshot_wire_limit_preserves_screen_and_complete_recent_lines() {
        let screen = b"authoritative-screen".to_vec();
        let mut snapshot = TerminalSnapshot {
            revision: Revision::new(9),
            size: TerminalSize::new(2, 20),
            active_screen: ActiveScreen::Main,
            screen_ansi: screen.clone(),
            recent_history_ansi: b"\x1b[mold-line\r\nrecent-one\r\nrecent-two\r\n".to_vec(),
            modes: TerminalModes::default(),
            scroll_metrics: None,
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
    fn debug_output_redacts_terminal_content() {
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
            scroll_metrics: Some(TerminalScrollMetrics {
                epoch: Revision::new(43),
                revision: Revision::new(47),
                offset_from_bottom: 0,
                max_offset_from_bottom: 5,
                viewport_rows: 41,
            }),
        };
        let delta = TerminalDelta {
            from_revision: Revision::new(47),
            to_revision: Revision::new(53),
            size: TerminalSize::new(41, 137),
            active_screen: ActiveScreen::Main,
            ansi: DELTA_SENTINEL.to_vec(),
            modes: TerminalModes::default(),
            scroll_metrics: None,
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
