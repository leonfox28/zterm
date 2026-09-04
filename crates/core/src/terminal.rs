//! Transport-neutral terminal domain values and reconnect payloads.
//!
//! The host-side parser and grid live in `zterm-terminal`. This module is kept
//! free of terminal-engine dependencies so protocol and future client crates
//! can share these values without compiling a host PTY stack.

use std::fmt;

use crate::{ResourceLimits, Revision};

/// Maximum number of side events returned by one terminal update.
pub const MAX_SIDE_EVENTS_PER_UPDATE: usize = 32;

/// Maximum number of source bytes retained for a title or icon-name event.
pub const MAX_TITLE_BYTES: usize = 256;

/// Maximum number of independently encoded rows in one history window.
pub const MAX_HISTORY_WINDOW_ROWS: usize = 240;

/// Maximum UTF-8 bytes carried by one semantic terminal cell.
pub const MAX_CELL_TEXT_BYTES: usize = 22;

/// Maximum UTF-8 bytes carried by one terminal clipboard write.
pub const MAX_TERMINAL_CLIPBOARD_BYTES: usize = 512 * 1024;

/// Failure to construct a bounded terminal clipboard write.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalClipboardError {
    /// Clipboard text must contain at least one byte.
    Empty,
    /// Clipboard text exceeds [`MAX_TERMINAL_CLIPBOARD_BYTES`].
    TooLarge,
    /// Clipboard text contains a NUL scalar.
    ContainsNul,
}

impl fmt::Display for TerminalClipboardError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "terminal clipboard text is empty",
            Self::TooLarge => "terminal clipboard text exceeds its byte limit",
            Self::ContainsNul => "terminal clipboard text contains NUL",
        })
    }
}

impl std::error::Error for TerminalClipboardError {}

/// Validated plain UTF-8 text for one transient terminal clipboard write.
#[derive(Clone, Eq, PartialEq)]
pub struct TerminalClipboardWrite(String);

impl TerminalClipboardWrite {
    /// Validates and owns one clipboard write.
    pub fn new(text: String) -> Result<Self, TerminalClipboardError> {
        if text.is_empty() {
            return Err(TerminalClipboardError::Empty);
        }
        if text.len() > MAX_TERMINAL_CLIPBOARD_BYTES {
            return Err(TerminalClipboardError::TooLarge);
        }
        if text.contains('\0') {
            return Err(TerminalClipboardError::ContainsNul);
        }
        Ok(Self(text))
    }

    /// Borrows the validated clipboard text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Moves the validated clipboard text out of this value.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Debug for TerminalClipboardWrite {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalClipboardWrite")
            .field("text", &"[REDACTED]")
            .field("text_len", &self.0.len())
            .finish()
    }
}

/// Transient host effect produced by terminal output.
#[derive(Clone, Eq, PartialEq)]
pub enum TerminalHostEffect {
    /// Replace the controlling attachment's system clipboard.
    ClipboardWrite(TerminalClipboardWrite),
}

impl fmt::Debug for TerminalHostEffect {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ClipboardWrite(value) => formatter
                .debug_tuple("ClipboardWrite")
                .field(value)
                .finish(),
        }
    }
}

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

/// Validated Kitty keyboard protocol flags requested by the hosted application.
#[derive(Clone, Copy, Default, Eq, PartialEq)]
pub struct TerminalKeyboardFlags(u8);

impl TerminalKeyboardFlags {
    /// Report ambiguous modified keys with CSI-u sequences.
    pub const DISAMBIGUATE_ESCAPE_CODES: Self = Self(1 << 0);
    /// Include press, repeat, and release event kinds.
    pub const REPORT_EVENT_TYPES: Self = Self(1 << 1);
    /// Include shifted and base-layout alternate key values.
    pub const REPORT_ALTERNATE_KEYS: Self = Self(1 << 2);
    /// Report every key as an escape sequence.
    pub const REPORT_ALL_KEYS_AS_ESCAPE_CODES: Self = Self(1 << 3);
    /// Include the text associated with a key event.
    pub const REPORT_ASSOCIATED_TEXT: Self = Self(1 << 4);
    /// All flags supported by the terminal domain.
    pub const ALL: Self = Self((1 << 5) - 1);

    /// Constructs a flag set, rejecting unknown bits.
    #[must_use]
    pub const fn from_bits(bits: u8) -> Option<Self> {
        if bits & !Self::ALL.0 == 0 {
            Some(Self(bits))
        } else {
            None
        }
    }

    /// Returns the wire-compatible flag bits.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Returns whether no keyboard enhancement is requested.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Returns whether every bit in `other` is enabled.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Returns the union of two validated flag sets.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl fmt::Debug for TerminalKeyboardFlags {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("TerminalKeyboardFlags")
            .field(&self.0)
            .finish()
    }
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
    /// Kitty keyboard protocol flags requested by the hosted application.
    pub keyboard_flags: TerminalKeyboardFlags,
}

/// One exact row in a semantic terminal surface.
#[derive(Clone, Eq, PartialEq)]
pub struct TerminalSurfaceRow {
    /// Exact cells from the first through the final terminal column.
    pub cells: Vec<TerminalCell>,
    /// Whether this row wraps logically into the following row.
    pub wrapped: bool,
}

impl fmt::Debug for TerminalSurfaceRow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalSurfaceRow")
            .field("cell_count", &self.cells.len())
            .field("wrapped", &self.wrapped)
            .finish()
    }
}

/// Complete semantic state of one active terminal screen.
#[derive(Clone, Eq, PartialEq)]
pub struct TerminalSurface {
    /// Exact rectangular viewport size.
    pub size: TerminalSize,
    /// Currently active child screen.
    pub active_screen: ActiveScreen,
    /// Exact rows from top to bottom.
    pub rows: Vec<TerminalSurfaceRow>,
    /// Child cursor state.
    pub cursor: TerminalCursor,
    /// Child-declared input and presentation modes.
    pub modes: TerminalModes,
    /// Live main-screen history extent, absent on the alternate screen.
    pub scroll_metrics: Option<TerminalScrollMetrics>,
}

impl fmt::Debug for TerminalSurface {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalSurface")
            .field("size", &self.size)
            .field("active_screen", &self.active_screen)
            .field("row_count", &self.rows.len())
            .field("cursor", &self.cursor)
            .field("modes", &self.modes)
            .field("scroll_metrics", &self.scroll_metrics)
            .finish()
    }
}

/// Full semantic reconnect state at one authoritative revision.
#[derive(Clone, Eq, PartialEq)]
pub struct TerminalSurfaceSnapshot {
    /// Revision represented by `surface`.
    pub revision: Revision,
    /// Complete exact active-screen surface.
    pub surface: TerminalSurface,
}

impl fmt::Debug for TerminalSurfaceSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalSurfaceSnapshot")
            .field("revision", &self.revision)
            .field("surface", &self.surface)
            .finish()
    }
}

/// Complete replacement of one semantic surface row.
#[derive(Clone, Eq, PartialEq)]
pub struct TerminalSurfaceRowPatch {
    /// Zero-based row index in the active surface.
    pub row: u16,
    /// Exact replacement row.
    pub replacement: TerminalSurfaceRow,
}

impl fmt::Debug for TerminalSurfaceRowPatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalSurfaceRowPatch")
            .field("row", &self.row)
            .field("replacement", &self.replacement)
            .finish()
    }
}

/// Merged semantic update from one attachment checkpoint to the latest revision.
#[derive(Clone, Eq, PartialEq)]
pub struct TerminalSurfaceDelta {
    /// Checkpoint revision used as the baseline.
    pub from_revision: Revision,
    /// Latest model revision represented by this update.
    pub to_revision: Revision,
    /// Exact viewport size after the update.
    pub size: TerminalSize,
    /// Active screen after the update.
    pub active_screen: ActiveScreen,
    /// Sorted, unique complete row replacements.
    pub row_patches: Vec<TerminalSurfaceRowPatch>,
    /// Cursor state after the update.
    pub cursor: TerminalCursor,
    /// Child-declared modes after the update.
    pub modes: TerminalModes,
    /// Live main-screen history extent, absent on the alternate screen.
    pub scroll_metrics: Option<TerminalScrollMetrics>,
}

impl fmt::Debug for TerminalSurfaceDelta {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalSurfaceDelta")
            .field("from_revision", &self.from_revision)
            .field("to_revision", &self.to_revision)
            .field("size", &self.size)
            .field("active_screen", &self.active_screen)
            .field("row_patch_count", &self.row_patches.len())
            .field("cursor", &self.cursor)
            .field("modes", &self.modes)
            .field("scroll_metrics", &self.scroll_metrics)
            .finish()
    }
}

/// Result of comparing a semantic checkpoint with the latest terminal state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerminalSurfaceDeltaResult {
    /// A compatible semantic update can be applied transactionally.
    Delta(TerminalSurfaceDelta),
    /// The checkpoint is incompatible and a complete surface replaces it.
    Resync(TerminalSurfaceSnapshot),
}

/// Structural failure in an exact semantic terminal surface.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalSurfaceError {
    /// A dimension is zero or exceeds the product viewport bound.
    InvalidSize,
    /// The number of rows does not equal the declared height.
    InvalidRowCount,
    /// A row does not contain exactly the declared number of columns.
    InvalidColumnCount,
    /// Cell text is oversized or contains a control character.
    InvalidCellText,
    /// Wide-head and continuation cells are not an exact adjacent pair.
    InvalidWidePair,
    /// The cursor lies outside the declared surface.
    InvalidCursor,
    /// Main/alternate scroll metrics disagree with the surface revision or size.
    InvalidScrollMetrics,
    /// A delta does not advance exactly from an older revision.
    InvalidRevision,
    /// Delta row patches are not sorted, unique, and in bounds.
    InvalidRowPatch,
    /// A delta baseline does not match the retained complete surface.
    IncompatibleBaseline,
    /// A semantic history window does not match its originating query.
    InvalidHistoryWindow,
}

impl fmt::Display for TerminalSurfaceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let detail = match self {
            Self::InvalidSize => "terminal surface size is outside product bounds",
            Self::InvalidRowCount => "terminal surface row count does not match its height",
            Self::InvalidColumnCount => {
                "terminal surface row width does not match its declared columns"
            }
            Self::InvalidCellText => "terminal surface cell text is invalid",
            Self::InvalidWidePair => "terminal surface wide-cell pairing is invalid",
            Self::InvalidCursor => "terminal surface cursor is outside its viewport",
            Self::InvalidScrollMetrics => "terminal surface scroll metrics are inconsistent",
            Self::InvalidRevision => "terminal surface delta revision does not advance",
            Self::InvalidRowPatch => "terminal surface row patch is invalid",
            Self::IncompatibleBaseline => "terminal surface delta baseline is incompatible",
            Self::InvalidHistoryWindow => {
                "semantic terminal history window does not match its request"
            }
        };
        formatter.write_str(detail)
    }
}

impl std::error::Error for TerminalSurfaceError {}

impl TerminalSurface {
    /// Validates a complete surface against its authoritative revision.
    pub fn validate(&self, revision: Revision) -> Result<(), TerminalSurfaceError> {
        validate_surface_size(self.size)?;
        if self.rows.len() != usize::from(self.size.rows) {
            return Err(TerminalSurfaceError::InvalidRowCount);
        }
        for row in &self.rows {
            validate_surface_row(row, self.size.columns)?;
        }
        if self.cursor.row >= self.size.rows || self.cursor.column >= self.size.columns {
            return Err(TerminalSurfaceError::InvalidCursor);
        }
        validate_surface_metrics(self.active_screen, self.size, revision, self.scroll_metrics)
    }
}

impl TerminalSurfaceSnapshot {
    /// Validates the complete semantic reconnect state.
    pub fn validate(&self) -> Result<(), TerminalSurfaceError> {
        self.surface.validate(self.revision)
    }
}

impl TerminalSurfaceDelta {
    /// Validates metadata and every complete row replacement.
    pub fn validate(&self) -> Result<(), TerminalSurfaceError> {
        if self.from_revision >= self.to_revision {
            return Err(TerminalSurfaceError::InvalidRevision);
        }
        validate_surface_size(self.size)?;
        if self.cursor.row >= self.size.rows || self.cursor.column >= self.size.columns {
            return Err(TerminalSurfaceError::InvalidCursor);
        }
        validate_surface_metrics(
            self.active_screen,
            self.size,
            self.to_revision,
            self.scroll_metrics,
        )?;
        let mut previous = None;
        for patch in &self.row_patches {
            if patch.row >= self.size.rows || previous.is_some_and(|row| row >= patch.row) {
                return Err(TerminalSurfaceError::InvalidRowPatch);
            }
            validate_surface_row(&patch.replacement, self.size.columns)?;
            previous = Some(patch.row);
        }
        Ok(())
    }

    /// Applies this delta to one complete baseline, committing only on success.
    pub fn apply_to(
        &self,
        baseline_revision: Revision,
        baseline: &mut TerminalSurface,
    ) -> Result<(), TerminalSurfaceError> {
        self.validate()?;
        if baseline_revision != self.from_revision
            || baseline.size != self.size
            || baseline.active_screen != self.active_screen
        {
            return Err(TerminalSurfaceError::IncompatibleBaseline);
        }
        let mut candidate = baseline.clone();
        for patch in &self.row_patches {
            candidate.rows[usize::from(patch.row)] = patch.replacement.clone();
        }
        candidate.cursor = self.cursor;
        candidate.modes = self.modes;
        candidate.scroll_metrics = self.scroll_metrics;
        candidate.validate(self.to_revision)?;
        *baseline = candidate;
        Ok(())
    }
}

fn validate_surface_size(size: TerminalSize) -> Result<(), TerminalSurfaceError> {
    let limits = ResourceLimits::default();
    if size.rows == 0
        || size.columns == 0
        || size.rows > limits.max_viewport_rows
        || size.columns > limits.max_viewport_columns
    {
        return Err(TerminalSurfaceError::InvalidSize);
    }
    Ok(())
}

fn validate_surface_row(
    row: &TerminalSurfaceRow,
    columns: u16,
) -> Result<(), TerminalSurfaceError> {
    if row.cells.len() != usize::from(columns) {
        return Err(TerminalSurfaceError::InvalidColumnCount);
    }
    for (column, cell) in row.cells.iter().enumerate() {
        if cell.contents.len() > MAX_CELL_TEXT_BYTES || cell.contents.chars().any(char::is_control)
        {
            return Err(TerminalSurfaceError::InvalidCellText);
        }
        if cell.wide && cell.wide_continuation {
            return Err(TerminalSurfaceError::InvalidWidePair);
        }
        if cell.wide {
            if cell.contents.is_empty()
                || row
                    .cells
                    .get(column + 1)
                    .is_none_or(|next| !next.wide_continuation || next.wide)
            {
                return Err(TerminalSurfaceError::InvalidWidePair);
            }
        } else if cell.wide_continuation
            && (!cell.contents.is_empty()
                || column == 0
                || row
                    .cells
                    .get(column - 1)
                    .is_none_or(|previous| !previous.wide || previous.wide_continuation))
        {
            return Err(TerminalSurfaceError::InvalidWidePair);
        }
    }
    Ok(())
}

fn validate_surface_metrics(
    screen: ActiveScreen,
    size: TerminalSize,
    revision: Revision,
    metrics: Option<TerminalScrollMetrics>,
) -> Result<(), TerminalSurfaceError> {
    match (screen, metrics) {
        (ActiveScreen::Main, Some(metrics))
            if metrics.is_valid()
                && metrics.revision == revision
                && metrics.offset_from_bottom == 0
                && metrics.viewport_rows == size.rows =>
        {
            Ok(())
        }
        (ActiveScreen::Alternate, None) => Ok(()),
        _ => Err(TerminalSurfaceError::InvalidScrollMetrics),
    }
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

/// Whether a complete history viewport preserved its previous row identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalViewportDisposition {
    /// The requested action resolved within the same retained-history identity.
    Exact,
    /// Retained-history identity changed and the closest current frame replaced it.
    Rebased,
}

/// Immutable coordinate-space identity for one main-screen history window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalHistoryWindowAnchor {
    /// Epoch which changes whenever retained row identity may have changed.
    pub epoch: Revision,
    /// Model revision observed while producing this anchor.
    pub revision: Revision,
    /// Largest retained logical row offset available in this epoch.
    pub max_offset_from_bottom: u64,
    /// Complete terminal size whose row coordinates this anchor describes.
    pub viewport: TerminalSize,
}

impl TerminalHistoryWindowAnchor {
    /// Returns whether this anchor is structurally usable.
    #[must_use]
    pub const fn is_valid(self) -> bool {
        self.viewport.rows > 0
            && self.viewport.columns > 0
            && self.epoch.get() <= self.revision.get()
    }
}

/// Stateless request for one bounded contiguous history-and-live row window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalHistoryWindowQuery {
    /// Client's last authoritative coordinate-space anchor.
    pub anchor: TerminalHistoryWindowAnchor,
    /// Absolute target offset expressed in the supplied anchor's coordinates.
    pub target_offset_from_bottom: u64,
    /// Additional rows requested toward older history.
    pub older_margin_rows: u16,
    /// Additional rows requested toward the live bottom.
    pub newer_margin_rows: u16,
}

impl TerminalHistoryWindowQuery {
    /// Returns whether the query obeys all transport-independent bounds.
    #[must_use]
    pub fn is_valid(self) -> bool {
        let rows = u32::from(self.anchor.viewport.rows);
        self.anchor.is_valid()
            && self.target_offset_from_bottom <= self.anchor.max_offset_from_bottom
            && u32::from(self.older_margin_rows)
                .checked_add(u32::from(self.newer_margin_rows))
                .is_some_and(|margins| margins <= rows.saturating_mul(2))
    }

    /// Resolves the only valid response shape for one immutable anchor.
    ///
    /// A response may advance from the request anchor, but it may never
    /// predate it. The returned range includes the complete target viewport
    /// plus the requested margins, clipped to retained-history and live-screen
    /// bounds.
    #[must_use]
    pub fn response_shape(
        self,
        response_anchor: TerminalHistoryWindowAnchor,
    ) -> Option<TerminalHistoryWindowResponseShape> {
        if !self.is_valid()
            || !response_anchor.is_valid()
            || response_anchor.revision < self.anchor.revision
        {
            return None;
        }

        let exact = self.anchor.epoch == response_anchor.epoch
            && self.anchor.viewport == response_anchor.viewport
            && self.anchor.max_offset_from_bottom <= response_anchor.max_offset_from_bottom;
        let disposition = if exact {
            TerminalViewportDisposition::Exact
        } else {
            TerminalViewportDisposition::Rebased
        };
        let target_offset_from_bottom = if exact {
            let growth = response_anchor
                .max_offset_from_bottom
                .checked_sub(self.anchor.max_offset_from_bottom)?;
            self.target_offset_from_bottom
                .checked_add(growth)?
                .min(response_anchor.max_offset_from_bottom)
        } else {
            self.target_offset_from_bottom
                .min(response_anchor.max_offset_from_bottom)
        };

        let history = i64::try_from(response_anchor.max_offset_from_bottom).ok()?;
        let target = i64::try_from(target_offset_from_bottom).ok()?;
        let visible_start = target.checked_neg()?;
        let visible_end = visible_start.checked_add(i64::from(response_anchor.viewport.rows))?;
        let first_row_from_live_top = visible_start
            .checked_sub(i64::from(self.older_margin_rows))?
            .max(history.checked_neg()?);
        let end_row_exclusive = visible_end
            .checked_add(i64::from(self.newer_margin_rows))?
            .min(i64::from(response_anchor.viewport.rows));
        let row_count =
            usize::try_from(end_row_exclusive.checked_sub(first_row_from_live_top)?).ok()?;
        if row_count < usize::from(response_anchor.viewport.rows)
            || row_count > MAX_HISTORY_WINDOW_ROWS
        {
            return None;
        }

        Some(TerminalHistoryWindowResponseShape {
            disposition,
            target_offset_from_bottom,
            first_row_from_live_top,
            row_count,
        })
    }
}

/// Request-bound metadata for one valid contiguous history-window response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalHistoryWindowResponseShape {
    /// Whether retained row identity is exact or rebased.
    pub disposition: TerminalViewportDisposition,
    /// Target translated into the response anchor's coordinates.
    pub target_offset_from_bottom: u64,
    /// First returned row relative to the response live-screen top.
    pub first_row_from_live_top: i64,
    /// Exact number of contiguous rows required by the request margins.
    pub row_count: usize,
}

/// One complete semantic history window produced from a single model revision.
#[derive(Clone, Eq, PartialEq)]
pub struct TerminalSurfaceHistoryWindowFrame {
    /// Whether the request retained or replaced its supplied row identity.
    pub disposition: TerminalViewportDisposition,
    /// Current authoritative coordinate-space anchor.
    pub anchor: TerminalHistoryWindowAnchor,
    /// Resolved target offset in the response anchor's coordinates.
    pub target_offset_from_bottom: u64,
    /// Coordinate of the first row relative to the current live-screen top.
    pub first_row_from_live_top: i64,
    /// Exact semantic rows from top to bottom.
    pub rows: Vec<TerminalSurfaceRow>,
}

impl TerminalSurfaceHistoryWindowFrame {
    /// Validates this frame as the exact semantic response to `query`.
    pub fn validate_for(
        &self,
        query: TerminalHistoryWindowQuery,
    ) -> Result<(), TerminalSurfaceError> {
        validate_surface_size(self.anchor.viewport)?;
        let Some(shape) = query.response_shape(self.anchor) else {
            return Err(TerminalSurfaceError::InvalidHistoryWindow);
        };
        if self.disposition != shape.disposition
            || self.target_offset_from_bottom != shape.target_offset_from_bottom
            || self.first_row_from_live_top != shape.first_row_from_live_top
            || self.rows.len() != shape.row_count
            || self.rows.len() > MAX_HISTORY_WINDOW_ROWS
        {
            return Err(TerminalSurfaceError::InvalidHistoryWindow);
        }
        for row in &self.rows {
            validate_surface_row(row, self.anchor.viewport.columns)?;
        }
        Ok(())
    }
}

impl fmt::Debug for TerminalSurfaceHistoryWindowFrame {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalSurfaceHistoryWindowFrame")
            .field("disposition", &self.disposition)
            .field("anchor", &self.anchor)
            .field("target_offset_from_bottom", &self.target_offset_from_bottom)
            .field("first_row_from_live_top", &self.first_row_from_live_top)
            .field("row_count", &self.rows.len())
            .finish()
    }
}

/// Result of one stateless semantic main-screen history-window projection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerminalSurfaceHistoryWindowResult {
    /// A complete continuous semantic window is available.
    Frame(TerminalSurfaceHistoryWindowFrame),
    /// Main-screen history cannot currently be projected.
    HistoryChanged {
        /// Current retained-history epoch.
        epoch: Revision,
        /// Current model revision.
        revision: Revision,
    },
    /// The supplied anchor or query was structurally invalid or from the future.
    HistoryGap {
        /// Current retained-history epoch.
        epoch: Revision,
        /// Current model revision.
        revision: Revision,
    },
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
    /// Latest transient host effect produced by this operation.
    pub host_effect: Option<TerminalHostEffect>,
}

impl fmt::Debug for TerminalUpdate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalUpdate")
            .field("revision", &self.revision)
            .field("replies", &"[REDACTED]")
            .field("reply_len", &self.replies.len())
            .field("events", &self.events)
            .field("host_effect", &self.host_effect)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn semantic_surface(revision: Revision) -> TerminalSurface {
        TerminalSurface {
            size: TerminalSize::new(2, 3),
            active_screen: ActiveScreen::Main,
            rows: vec![
                TerminalSurfaceRow {
                    cells: vec![
                        TerminalCell {
                            contents: "界".to_owned(),
                            wide: true,
                            ..TerminalCell::default()
                        },
                        TerminalCell {
                            wide_continuation: true,
                            ..TerminalCell::default()
                        },
                        TerminalCell {
                            contents: "x".to_owned(),
                            ..TerminalCell::default()
                        },
                    ],
                    wrapped: true,
                },
                TerminalSurfaceRow {
                    cells: vec![TerminalCell::default(); 3],
                    wrapped: false,
                },
            ],
            cursor: TerminalCursor {
                row: 1,
                column: 2,
                visible: true,
                style: TerminalStyle::default(),
            },
            modes: TerminalModes::default(),
            scroll_metrics: Some(TerminalScrollMetrics {
                epoch: Revision::ZERO,
                revision,
                offset_from_bottom: 0,
                max_offset_from_bottom: 7,
                viewport_rows: 2,
            }),
        }
    }

    #[test]
    fn semantic_surface_validation_is_exact_content_free_and_transactional() {
        let revision = Revision::new(3);
        let surface = semantic_surface(revision);
        assert_eq!(surface.validate(revision), Ok(()));
        let snapshot = TerminalSurfaceSnapshot {
            revision,
            surface: surface.clone(),
        };
        assert_eq!(snapshot.validate(), Ok(()));

        let mut applied = surface.clone();
        let delta = TerminalSurfaceDelta {
            from_revision: revision,
            to_revision: Revision::new(4),
            size: surface.size,
            active_screen: surface.active_screen,
            row_patches: vec![TerminalSurfaceRowPatch {
                row: 1,
                replacement: TerminalSurfaceRow {
                    cells: vec![
                        TerminalCell {
                            contents: "new".to_owned(),
                            ..TerminalCell::default()
                        },
                        TerminalCell::default(),
                        TerminalCell::default(),
                    ],
                    wrapped: false,
                },
            }],
            cursor: surface.cursor,
            modes: surface.modes,
            scroll_metrics: Some(TerminalScrollMetrics {
                revision: Revision::new(4),
                ..surface.scroll_metrics.expect("metrics")
            }),
        };
        assert_eq!(delta.apply_to(revision, &mut applied), Ok(()));
        assert_eq!(applied.rows[1], delta.row_patches[0].replacement);

        let before = applied.clone();
        let mut malformed = delta.clone();
        malformed.row_patches[0].replacement.cells[0].contents = "bad\ncell".to_owned();
        assert_eq!(
            malformed.apply_to(Revision::new(4), &mut applied),
            Err(TerminalSurfaceError::InvalidCellText)
        );
        assert_eq!(
            applied, before,
            "a rejected patch must not partially commit"
        );
    }

    #[test]
    fn semantic_surface_rejects_shape_cursor_metrics_and_wide_pair_errors() {
        let revision = Revision::new(5);
        let mut surface = semantic_surface(revision);
        surface.rows[0].cells.pop();
        assert_eq!(
            surface.validate(revision),
            Err(TerminalSurfaceError::InvalidColumnCount)
        );

        let mut surface = semantic_surface(revision);
        surface.rows[0].cells[1].wide_continuation = false;
        assert_eq!(
            surface.validate(revision),
            Err(TerminalSurfaceError::InvalidWidePair)
        );

        let mut surface = semantic_surface(revision);
        surface.cursor.column = surface.size.columns;
        assert_eq!(
            surface.validate(revision),
            Err(TerminalSurfaceError::InvalidCursor)
        );

        let mut surface = semantic_surface(revision);
        surface.scroll_metrics.as_mut().expect("metrics").revision = Revision::new(4);
        assert_eq!(
            surface.validate(revision),
            Err(TerminalSurfaceError::InvalidScrollMetrics)
        );
    }

    #[test]
    fn semantic_history_window_is_request_bound_and_redacted() {
        const SENTINEL: &str = "SEM_WINDOW_57c1";
        let query = TerminalHistoryWindowQuery {
            anchor: TerminalHistoryWindowAnchor {
                epoch: Revision::new(1),
                revision: Revision::new(2),
                max_offset_from_bottom: 4,
                viewport: TerminalSize::new(2, 3),
            },
            target_offset_from_bottom: 1,
            older_margin_rows: 0,
            newer_margin_rows: 0,
        };
        let frame = TerminalSurfaceHistoryWindowFrame {
            disposition: TerminalViewportDisposition::Exact,
            anchor: query.anchor,
            target_offset_from_bottom: 1,
            first_row_from_live_top: -1,
            rows: vec![
                TerminalSurfaceRow {
                    cells: vec![
                        TerminalCell {
                            contents: SENTINEL.to_owned(),
                            ..TerminalCell::default()
                        },
                        TerminalCell::default(),
                        TerminalCell::default(),
                    ],
                    wrapped: false,
                },
                TerminalSurfaceRow {
                    cells: vec![TerminalCell::default(); 3],
                    wrapped: false,
                },
            ],
        };
        assert_eq!(frame.validate_for(query), Ok(()));
        let debug = format!("{frame:?}");
        assert!(!debug.contains(SENTINEL));
        assert!(debug.contains("row_count: 2"));
    }

    #[test]
    fn debug_output_redacts_terminal_content() {
        const CELL_SENTINEL: &str = "TERM_CELL_SENTINEL_7f0d";
        const TITLE_SENTINEL: &str = "TERM_TITLE_SENTINEL_154c";
        const ICON_SENTINEL: &str = "TERM_ICON_SENTINEL_861a";
        const REPLY_SENTINEL: &[u8] = b"TERM_REPLY_SENTINEL_23d9";
        const SURFACE_SENTINEL: &str = "TERM_SURFACE_SENTINEL_3ba7";
        const DELTA_SENTINEL: &str = "TERM_DELTA_SENTINEL_c156";

        let cell = TerminalCell {
            contents: CELL_SENTINEL.to_owned(),
            wide: true,
            wide_continuation: false,
            style: TerminalStyle {
                bold: true,
                ..TerminalStyle::default()
            },
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
            host_effect: Some(TerminalHostEffect::ClipboardWrite(
                TerminalClipboardWrite::new("TERM_CLIPBOARD_SENTINEL_98da".to_owned())
                    .expect("valid clipboard value"),
            )),
        };
        let mut surface = semantic_surface(Revision::new(47));
        surface.rows[1].cells[0].contents = SURFACE_SENTINEL.to_owned();
        let snapshot = TerminalSurfaceSnapshot {
            revision: Revision::new(47),
            surface,
        };
        let delta = TerminalSurfaceDelta {
            from_revision: Revision::new(47),
            to_revision: Revision::new(53),
            size: TerminalSize::new(2, 3),
            active_screen: ActiveScreen::Main,
            row_patches: vec![TerminalSurfaceRowPatch {
                row: 1,
                replacement: TerminalSurfaceRow {
                    cells: vec![
                        TerminalCell {
                            contents: DELTA_SENTINEL.to_owned(),
                            ..TerminalCell::default()
                        },
                        TerminalCell::default(),
                        TerminalCell::default(),
                    ],
                    wrapped: false,
                },
            }],
            cursor: snapshot.surface.cursor,
            modes: TerminalModes::default(),
            scroll_metrics: Some(TerminalScrollMetrics {
                revision: Revision::new(53),
                ..snapshot.surface.scroll_metrics.expect("metrics")
            }),
        };

        let rendered = format!(
            "{cell:?} {update:?} {snapshot:?} {delta:?} {:?} {:?}",
            TerminalSurfaceDeltaResult::Delta(delta.clone()),
            TerminalSurfaceDeltaResult::Resync(snapshot.clone()),
        );
        for text in [
            CELL_SENTINEL,
            TITLE_SENTINEL,
            ICON_SENTINEL,
            SURFACE_SENTINEL,
            DELTA_SENTINEL,
        ] {
            assert!(!rendered.contains(text));
        }
        assert!(!rendered.contains(std::str::from_utf8(REPLY_SENTINEL).expect("ASCII sentinel")));
        assert!(!rendered.contains(&format!("{REPLY_SENTINEL:?}")));
        assert!(!rendered.contains("TERM_CLIPBOARD_SENTINEL_98da"));
        assert!(rendered.contains("[REDACTED]"));
        assert!(rendered.contains(&format!("reply_len: {}", REPLY_SENTINEL.len())));
        assert!(rendered.contains("row_patch_count: 1"));

        assert_eq!(cell, cell.clone());
        assert_eq!(update, update.clone());
        assert_eq!(snapshot, snapshot.clone());
        assert_eq!(delta, delta.clone());
    }

    #[test]
    fn clipboard_and_keyboard_domain_values_enforce_exact_bounds() {
        assert_eq!(
            TerminalClipboardWrite::new(String::new()),
            Err(TerminalClipboardError::Empty)
        );
        assert_eq!(
            TerminalClipboardWrite::new("a\0b".to_owned()),
            Err(TerminalClipboardError::ContainsNul)
        );
        let exact = TerminalClipboardWrite::new("界".repeat(MAX_TERMINAL_CLIPBOARD_BYTES / 3))
            .expect("largest complete UTF-8 value below the byte cap");
        assert!(exact.as_str().len() <= MAX_TERMINAL_CLIPBOARD_BYTES);
        assert_eq!(
            TerminalClipboardWrite::new("x".repeat(MAX_TERMINAL_CLIPBOARD_BYTES + 1)),
            Err(TerminalClipboardError::TooLarge)
        );
        assert!(!format!("{exact:?}").contains('界'));

        assert_eq!(
            TerminalKeyboardFlags::from_bits(0x1f),
            Some(TerminalKeyboardFlags::ALL)
        );
        assert_eq!(TerminalKeyboardFlags::from_bits(0x20), None);
        assert!(TerminalKeyboardFlags::ALL.contains(TerminalKeyboardFlags::REPORT_ASSOCIATED_TEXT));
    }
}
