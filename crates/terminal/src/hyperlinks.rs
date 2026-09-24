//! Bounded link identities shared by engine cells and semantic projections.
use crate::engine::AlacrittyEngine;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::{Cell, Hyperlink};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use zterm_core::terminal::{
    MAX_TERMINAL_HYPERLINK_BYTES, MAX_TERMINAL_HYPERLINKS, TerminalHyperlink,
};

#[derive(Default)]
pub(crate) struct Hyperlinks {
    values: HashMap<Hyperlink, Arc<TerminalHyperlink>>,
    screens: [HashSet<Hyperlink>; 2],
    bytes: usize,
    next_id: u64,
}

impl AlacrittyEngine {
    pub(crate) fn close_hyperlink(&mut self) {
        self.term.grid_mut().cursor.template.set_hyperlink(None);
    }

    pub(crate) fn set_web_hyperlink(&mut self, payload: &str) -> bool {
        self.close_hyperlink();
        let Some((params, uri)) = payload.split_once(';') else {
            return false;
        };
        if uri.is_empty() {
            return true;
        }
        let id = params
            .split(':')
            .find_map(|p| p.strip_prefix("id="))
            .filter(|id| !id.is_empty());
        let generated;
        let id = if let Some(id) = id {
            id
        } else {
            let Some(next) = self.hyperlinks.next_id.checked_add(1) else {
                return false;
            };
            self.hyperlinks.next_id = next;
            generated = format!("zterm-auto-{next}");
            &generated
        };
        let Ok(value) = TerminalHyperlink::new(id, uri) else {
            return false;
        };
        let candidate = Hyperlink::new(Some(value.id()), value.uri().to_owned());
        if let Some((shared, _)) = self.hyperlinks.values.get_key_value(&candidate) {
            self.term
                .grid_mut()
                .cursor
                .template
                .set_hyperlink(Some(shared.clone()));
            return true;
        }
        if self.hyperlinks.values.len() >= MAX_TERMINAL_HYPERLINKS
            || self.hyperlinks.bytes + value.payload_bytes() > MAX_TERMINAL_HYPERLINK_BYTES
        {
            self.reconcile_hyperlinks();
        }
        if self.hyperlinks.values.len() >= MAX_TERMINAL_HYPERLINKS
            || self.hyperlinks.bytes + value.payload_bytes() > MAX_TERMINAL_HYPERLINK_BYTES
        {
            return false;
        }
        self.hyperlinks.bytes += value.payload_bytes();
        self.hyperlinks
            .values
            .insert(candidate.clone(), Arc::new(value));
        self.term
            .grid_mut()
            .cursor
            .template
            .set_hyperlink(Some(candidate));
        true
    }

    pub(crate) fn cell_hyperlink(&self, cell: &Cell) -> Option<Arc<TerminalHyperlink>> {
        self.hyperlinks.values.get(&cell.hyperlink()?).cloned()
    }

    pub(crate) fn reconcile_hyperlinks(&mut self) {
        let index =
            usize::from(self.active_screen() == zterm_core::terminal::ActiveScreen::Alternate);
        let grid = self.term.grid();
        let current = &mut self.hyperlinks.screens[index];
        current.clear();
        for line in -(grid.history_size() as i32)..grid.screen_lines() as i32 {
            for column in 0..grid.columns() {
                if let Some(link) = grid[Line(line)][Column(column)].hyperlink() {
                    current.insert(link);
                }
            }
        }
        current.extend(grid.cursor.template.hyperlink());
        current.extend(grid.saved_cursor.template.hyperlink());
        let screens = &self.hyperlinks.screens;
        self.hyperlinks
            .values
            .retain(|key, _| screens.iter().any(|s| s.contains(key)));
        self.hyperlinks.bytes = self
            .hyperlinks
            .values
            .values()
            .map(|v| v.payload_bytes())
            .sum();
    }
}
