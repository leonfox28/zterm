use std::sync::{Arc, Mutex, MutexGuard};

use alacritty_terminal::Term;
use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Osc52, TermMode};
use alacritty_terminal::vte::ansi::Processor;
use unicode_width::UnicodeWidthChar;
use zterm_core::terminal::{
    ActiveScreen, MAX_SIDE_EVENTS_PER_UPDATE, RejectedEffect, TerminalSideEvent, TerminalSize,
    UnsupportedSequenceKind,
};

use crate::{
    MAX_CELL_TEXT_BYTES, MAX_COMBINING_BYTES_PER_SESSION, MAX_COMBINING_CELLS_PER_SESSION,
};

#[derive(Clone, Copy)]
pub(crate) struct EngineSize {
    rows: usize,
    columns: usize,
}

impl EngineSize {
    pub(crate) fn new(size: TerminalSize) -> Self {
        Self {
            rows: usize::from(size.rows),
            columns: usize::from(size.columns),
        }
    }
}

impl Dimensions for EngineSize {
    fn total_lines(&self) -> usize {
        self.rows
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

#[derive(Clone, Copy)]
enum EngineEvent {
    Bell,
    EffectRejected(RejectedEffect),
    Unsupported(UnsupportedSequenceKind),
}

#[derive(Default)]
pub(crate) struct EngineEventBuffer {
    events: Vec<EngineEvent>,
    dropped: u64,
}

impl EngineEventBuffer {
    fn push(&mut self, event: EngineEvent) {
        if self.events.len() < MAX_SIDE_EVENTS_PER_UPDATE {
            self.events.push(event);
        } else {
            self.dropped = self.dropped.saturating_add(1);
        }
    }

    fn drain(&mut self) -> (Vec<EngineEvent>, u64) {
        (
            std::mem::take(&mut self.events),
            std::mem::take(&mut self.dropped),
        )
    }
}

#[derive(Clone, Default)]
pub(crate) struct BoundedEventSink {
    events: Arc<Mutex<EngineEventBuffer>>,
}

impl BoundedEventSink {
    fn lock(&self) -> MutexGuard<'_, EngineEventBuffer> {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn drain(&self) -> (Vec<EngineEvent>, u64) {
        self.lock().drain()
    }
}

impl EventListener for BoundedEventSink {
    fn send_event(&self, event: Event) {
        let mapped = match event {
            Event::Bell => Some(EngineEvent::Bell),
            Event::ClipboardStore(_, _) => {
                Some(EngineEvent::EffectRejected(RejectedEffect::ClipboardWrite))
            }
            Event::ClipboardLoad(_, _) => {
                Some(EngineEvent::EffectRejected(RejectedEffect::ClipboardRead))
            }
            Event::Title(_) | Event::ResetTitle => {
                Some(EngineEvent::Unsupported(UnsupportedSequenceKind::Osc))
            }
            Event::PtyWrite(_) | Event::ColorRequest(_, _) | Event::TextAreaSizeRequest(_) => {
                Some(EngineEvent::Unsupported(UnsupportedSequenceKind::Csi))
            }
            Event::MouseCursorDirty
            | Event::CursorBlinkingChange
            | Event::Wakeup
            | Event::Exit
            | Event::ChildExit(_) => None,
        };
        if let Some(event) = mapped {
            self.lock().push(event);
        }
    }
}

#[derive(Clone, Copy, Default)]
struct ScreenCombiningBudget {
    bytes: usize,
    cells: usize,
}

#[derive(Default)]
struct CombiningBudget {
    screens: [ScreenCombiningBudget; 2],
}

impl CombiningBudget {
    const fn screen_index(screen: ActiveScreen) -> usize {
        match screen {
            ActiveScreen::Main => 0,
            ActiveScreen::Alternate => 1,
        }
    }

    #[cfg(test)]
    fn screen(&self, screen: ActiveScreen) -> ScreenCombiningBudget {
        self.screens[Self::screen_index(screen)]
    }

    fn screen_mut(&mut self, screen: ActiveScreen) -> &mut ScreenCombiningBudget {
        &mut self.screens[Self::screen_index(screen)]
    }

    fn total_bytes(&self) -> usize {
        self.screens
            .iter()
            .fold(0, |total, screen| total.saturating_add(screen.bytes))
    }

    fn total_cells(&self) -> usize {
        self.screens
            .iter()
            .fold(0, |total, screen| total.saturating_add(screen.cells))
    }
}

pub(crate) struct AlacrittyEngine {
    processor: Processor,
    term: Term<BoundedEventSink>,
    sink: BoundedEventSink,
    legacy_x10_mouse: bool,
    combining: CombiningBudget,
}

impl AlacrittyEngine {
    pub(crate) fn new(size: TerminalSize, scrollback_rows: usize) -> Self {
        let sink = BoundedEventSink::default();
        let config = Config {
            scrolling_history: scrollback_rows,
            kitty_keyboard: false,
            osc52: Osc52::Disabled,
            ..Config::default()
        };
        let mut engine = Self {
            processor: Processor::new(),
            term: Term::new(config, &EngineSize::new(size), sink.clone()),
            sink,
            legacy_x10_mouse: false,
            combining: CombiningBudget::default(),
        };
        // Alacritty enables alternate-scroll by default, while Zterm's public
        // contract starts with all optional input modes disabled.
        engine.feed_raw(b"\x1b[?1007l");
        let _ = engine.take_events();
        engine
    }

    pub(crate) fn feed_raw(&mut self, bytes: &[u8]) {
        let previous_screen = self.active_screen();
        self.processor.advance(&mut self.term, bytes);
        if self.active_screen() != previous_screen {
            self.reconcile_active_combining_budget();
        }
    }

    pub(crate) fn feed_screen_transition(&mut self, bytes: &[u8]) {
        self.reconcile_active_combining_budget();
        self.processor.advance(&mut self.term, bytes);
        self.reconcile_active_combining_budget();
    }

    pub(crate) fn feed_reset(&mut self, bytes: &[u8]) {
        self.processor.advance(&mut self.term, bytes);
        self.legacy_x10_mouse = false;
        self.combining = CombiningBudget::default();
        self.reconcile_active_combining_budget();
    }

    pub(crate) fn resize(&mut self, size: TerminalSize) {
        self.reconcile_active_combining_budget();
        self.term.resize(EngineSize::new(size));
        self.reconcile_active_combining_budget();
    }

    pub(crate) fn size(&self) -> TerminalSize {
        TerminalSize::new(
            u16::try_from(self.term.screen_lines()).unwrap_or(u16::MAX),
            u16::try_from(self.term.columns()).unwrap_or(u16::MAX),
        )
    }

    pub(crate) fn term(&self) -> &Term<BoundedEventSink> {
        &self.term
    }

    pub(crate) fn active_screen(&self) -> ActiveScreen {
        if self.term.mode().contains(TermMode::ALT_SCREEN) {
            ActiveScreen::Alternate
        } else {
            ActiveScreen::Main
        }
    }

    pub(crate) fn set_legacy_x10_mouse(&mut self, enabled: bool) {
        self.legacy_x10_mouse = enabled;
    }

    pub(crate) const fn legacy_x10_mouse(&self) -> bool {
        self.legacy_x10_mouse
    }

    pub(crate) fn cursor_report(&self, private: bool) -> Vec<u8> {
        let point = self.term.grid().cursor.point;
        let rows = self.term.screen_lines();
        let columns = self.term.columns();
        let row = usize::try_from(point.line.0)
            .unwrap_or_default()
            .min(rows.saturating_sub(1))
            .saturating_add(1);
        let column = point
            .column
            .0
            .min(columns.saturating_sub(1))
            .saturating_add(1);
        let marker = if private { "?" } else { "" };
        format!("\x1b[{marker}{row};{column}R").into_bytes()
    }

    pub(crate) fn accept_character(&mut self, character: char) -> bool {
        if UnicodeWidthChar::width(character) != Some(0) {
            return true;
        }

        let (existing_bytes, creates_cell) = self.combining_target();
        let character_bytes = character.len_utf8();
        if existing_bytes.saturating_add(character_bytes) > MAX_CELL_TEXT_BYTES {
            return false;
        }

        if self.combining_limit_exceeded(character_bytes, creates_cell) {
            // Ordinary printable input may have overwritten or evicted cells
            // since the last accounting update. Recount only at the boundary
            // where stale conservative usage would otherwise reject input.
            self.reconcile_active_combining_budget();
            if self.combining_limit_exceeded(character_bytes, creates_cell) {
                return false;
            }
        }

        let screen = self.active_screen();
        let budget = self.combining.screen_mut(screen);
        budget.bytes += character_bytes;
        if creates_cell {
            budget.cells += 1;
        }
        true
    }

    pub(crate) fn take_events(&self) -> Vec<TerminalSideEvent> {
        let (events, mut dropped) = self.sink.drain();
        let mut events = events
            .into_iter()
            .map(|event| match event {
                EngineEvent::Bell => TerminalSideEvent::AudibleBell,
                EngineEvent::EffectRejected(effect) => TerminalSideEvent::EffectRejected(effect),
                EngineEvent::Unsupported(kind) => TerminalSideEvent::UnsupportedSequence(kind),
            })
            .collect::<Vec<_>>();
        if dropped > 0 {
            if events.len() == MAX_SIDE_EVENTS_PER_UPDATE {
                events.pop();
                dropped = dropped.saturating_add(1);
            }
            events.push(TerminalSideEvent::EventsDropped { count: dropped });
        }
        events
    }

    #[cfg(test)]
    pub(crate) fn lifecycle_probe(&self) -> std::sync::Weak<Mutex<EngineEventBuffer>> {
        Arc::downgrade(&self.sink.events)
    }

    fn combining_target(&self) -> (usize, bool) {
        let grid = self.term.grid();
        let mut column = grid.cursor.point.column;
        if !grid.cursor.input_needs_wrap {
            column.0 = column.0.saturating_sub(1);
        }
        let line = grid.cursor.point.line;
        if grid[line][column].flags.contains(Flags::WIDE_CHAR_SPACER) {
            column.0 = column.0.saturating_sub(1);
        }
        let cell = &grid[line][Column(column.0)];
        let existing_bytes = cell.c.len_utf8()
            + cell
                .zerowidth()
                .unwrap_or_default()
                .iter()
                .map(|value| value.len_utf8())
                .sum::<usize>();
        let creates_cell = cell.zerowidth().is_none_or(<[char]>::is_empty);
        (existing_bytes, creates_cell)
    }

    fn combining_limit_exceeded(&self, character_bytes: usize, creates_cell: bool) -> bool {
        self.combining.total_bytes().saturating_add(character_bytes)
            > MAX_COMBINING_BYTES_PER_SESSION
            || (creates_cell && self.combining.total_cells() >= MAX_COMBINING_CELLS_PER_SESSION)
    }

    fn reconcile_active_combining_budget(&mut self) {
        let screen = self.active_screen();
        let grid = self.term.grid();
        let history = i32::try_from(grid.history_size()).unwrap_or(i32::MAX);
        let screen_lines = i32::try_from(grid.screen_lines()).unwrap_or(i32::MAX);
        let mut usage = ScreenCombiningBudget::default();
        for line in -history..screen_lines {
            for column in 0..grid.columns() {
                let zerowidth = grid[Line(line)][Column(column)]
                    .zerowidth()
                    .unwrap_or_default();
                if !zerowidth.is_empty() {
                    usage.cells = usage.cells.saturating_add(1);
                    usage.bytes = zerowidth.iter().fold(usage.bytes, |bytes, character| {
                        bytes.saturating_add(character.len_utf8())
                    });
                }
            }
        }
        *self.combining.screen_mut(screen) = usage;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combining_cell_and_total_byte_budgets_are_independent_and_bounded() {
        let mut engine = AlacrittyEngine::new(TerminalSize::new(80, 80), 0);
        for index in 0..MAX_COMBINING_CELLS_PER_SESSION {
            let row = index / 80 + 1;
            let column = index % 80 + 1;
            engine.feed_raw(format!("\x1b[{row};{column}He").as_bytes());
            assert!(engine.accept_character('\u{301}'));
            engine.feed_raw("\u{301}".as_bytes());
        }
        engine.feed_raw(b"\x1b[80;80He");
        assert!(!engine.accept_character('\u{302}'));
        assert_eq!(
            engine.combining.total_cells(),
            MAX_COMBINING_CELLS_PER_SESSION
        );

        let mut bytes = AlacrittyEngine::new(TerminalSize::new(80, 80), 0);
        let marks_per_cell = MAX_COMBINING_BYTES_PER_SESSION
            / MAX_COMBINING_CELLS_PER_SESSION
            / '\u{301}'.len_utf8();
        for index in 0..MAX_COMBINING_CELLS_PER_SESSION {
            let row = index / 80 + 1;
            let column = index % 80 + 1;
            bytes.feed_raw(format!("\x1b[{row};{column}He").as_bytes());
            for _ in 0..marks_per_cell {
                assert!(bytes.accept_character('\u{301}'));
                bytes.feed_raw("\u{301}".as_bytes());
            }
        }
        bytes.feed_raw(b"\x1b[1;1H");
        assert!(!bytes.accept_character('\u{302}'));
        assert_eq!(
            bytes.combining.total_bytes(),
            MAX_COMBINING_BYTES_PER_SESSION
        );
    }

    #[test]
    fn combining_usage_is_tracked_for_both_screens() {
        let mut engine = AlacrittyEngine::new(TerminalSize::new(2, 8), 0);
        engine.feed_raw(b"e");
        assert!(engine.accept_character('\u{301}'));
        engine.feed_raw("\u{301}".as_bytes());
        engine.feed_screen_transition(b"\x1b[?1049h");
        assert_eq!(engine.combining.screen(ActiveScreen::Main).cells, 1);
        assert_eq!(engine.combining.screen(ActiveScreen::Alternate).cells, 0);

        engine.feed_raw(b"a");
        assert!(engine.accept_character('\u{302}'));
        engine.feed_raw("\u{302}".as_bytes());
        engine.feed_screen_transition(b"\x1b[?1049l");
        assert_eq!(engine.combining.screen(ActiveScreen::Main).cells, 1);
        assert_eq!(engine.combining.screen(ActiveScreen::Alternate).cells, 1);
        assert_eq!(engine.combining.total_cells(), 2);

        engine.feed_screen_transition(b"\x1b[?1049h");
        assert_eq!(engine.combining.screen(ActiveScreen::Main).cells, 1);
        assert_eq!(engine.combining.screen(ActiveScreen::Alternate).cells, 0);
        assert_eq!(engine.combining.total_cells(), 1);
    }

    #[test]
    fn resize_and_history_eviction_reclaim_conservative_combining_usage() {
        let mut overwritten = AlacrittyEngine::new(TerminalSize::new(2, 8), 0);
        overwritten.feed_raw(b"e");
        assert!(overwritten.accept_character('\u{301}'));
        overwritten.feed_raw("\u{301}".as_bytes());
        overwritten.feed_raw(b"\x1b[1;1Hx");
        assert_eq!(overwritten.combining.total_cells(), 1);
        overwritten.resize(TerminalSize::new(2, 8));
        assert_eq!(overwritten.combining.total_cells(), 0);

        let mut evicted = AlacrittyEngine::new(TerminalSize::new(1, 8), 1);
        evicted.feed_raw(b"e");
        assert!(evicted.accept_character('\u{301}'));
        evicted.feed_raw("\u{301}".as_bytes());
        evicted.feed_raw(b"\r\none\r\ntwo");
        evicted.resize(TerminalSize::new(1, 8));
        assert_eq!(evicted.combining.total_cells(), 0);
    }

    #[test]
    fn upstream_event_sink_is_bounded_before_model_collection() {
        let engine = AlacrittyEngine::new(TerminalSize::new(2, 8), 0);
        for _ in 0..100 {
            engine.sink.send_event(Event::ResetTitle);
        }
        let events = engine.take_events();
        assert_eq!(events.len(), MAX_SIDE_EVENTS_PER_UPDATE);
        assert_eq!(
            events.last(),
            Some(&TerminalSideEvent::EventsDropped { count: 69 })
        );
    }
}
