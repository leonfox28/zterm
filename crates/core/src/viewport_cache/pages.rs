//! Accounted immutable pages; request correlation remains in ViewportCache.
use super::*;
use std::{collections::VecDeque, mem::size_of, sync::Arc};

/// Multi-page limits include retained windows and pinned selection sources.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViewportCacheBudget {
    /// Maximum retained rows, including overlapping independently projected rows.
    pub rows: usize,
    /// Maximum accounted allocation bytes, including nested row allocations.
    pub bytes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Page<Row> {
    window: Arc<CachedViewportWindow<Row>>,
    bytes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Pages<Row> {
    budget: ViewportCacheBudget,
    entries: VecDeque<Page<Row>>,
}
impl<Row> Pages<Row> {
    pub(super) fn new(budget: ViewportCacheBudget) -> Self {
        Self {
            budget,
            entries: VecDeque::new(),
        }
    }
    pub(super) fn usage(&self) -> ViewportCacheBudget {
        // Account reserved metadata too; shrinking on eviction bounds its capacity.
        let mut total = ViewportCacheBudget {
            rows: 0,
            bytes: size_of::<Self>() + self.entries.capacity() * size_of::<Page<Row>>(),
        };
        for page in &self.entries {
            total.rows = total.rows.saturating_add(page.window.rows.len());
            total.bytes = total.bytes.saturating_add(page.bytes);
        }
        total
    }
    pub(super) fn find(
        &self,
        latest: TerminalHistoryWindowAnchor,
        target: u64,
    ) -> Option<(&CachedViewportWindow<Row>, u64)> {
        self.entries.iter().rev().find_map(|page| {
            let offset = reusable_target(latest, &page.window, target)?;
            Some((&*page.window, offset))
        })
    }
    pub(super) fn touch(&mut self, latest: TerminalHistoryWindowAnchor, target: u64) {
        if let Some(index) = self
            .entries
            .iter()
            .rposition(|page| reusable_target(latest, &page.window, target).is_some())
        {
            let page = self.entries.remove(index).expect("index remains present");
            self.entries.push_back(page);
        }
    }
    pub(super) fn pin(
        &self,
        latest: TerminalHistoryWindowAnchor,
        target: u64,
    ) -> Option<Arc<CachedViewportWindow<Row>>> {
        self.entries
            .iter()
            .rev()
            .find(|page| reusable_target(latest, &page.window, target).is_some())
            .map(|page| Arc::clone(&page.window))
    }
    pub(super) fn discard_unpinned(&mut self) {
        self.entries
            .retain(|page| Arc::strong_count(&page.window) > 1);
        self.entries.shrink_to_fit();
    }
    pub(super) fn insert(
        &mut self,
        window: CachedViewportWindow<Row>,
        nested_bytes: usize,
    ) -> bool {
        // Row's inline representation is in its Vec; the caller accounts its
        // nested heap allocations. Arc metadata and window metadata count once.
        let bytes = size_of::<CachedViewportWindow<Row>>()
            .saturating_add(2 * size_of::<usize>())
            .saturating_add(window.rows.capacity().saturating_mul(size_of::<Row>()))
            .saturating_add(nested_bytes);
        if window.rows.len() > self.budget.rows
            || bytes.saturating_add(size_of::<Page<Row>>()) > self.budget.bytes
        {
            return false;
        }
        loop {
            let used = self.usage();
            let extra_metadata = if self.entries.len() == self.entries.capacity() {
                size_of::<Page<Row>>()
            } else {
                0
            };
            if used.rows.saturating_add(window.rows.len()) <= self.budget.rows
                && used
                    .bytes
                    .saturating_add(bytes)
                    .saturating_add(extra_metadata)
                    <= self.budget.bytes
            {
                break;
            }
            let Some(index) = self
                .entries
                .iter()
                .position(|page| Arc::strong_count(&page.window) == 1)
            else {
                return false;
            };
            self.entries.remove(index);
            self.entries.shrink_to_fit();
        }
        // reserve_exact avoids geometric metadata capacity escaping accounting.
        self.entries.reserve_exact(1);
        self.entries.push_back(Page {
            window: Arc::new(window),
            bytes,
        });
        true
    }
}

fn reusable_target<Row>(
    latest: TerminalHistoryWindowAnchor,
    window: &CachedViewportWindow<Row>,
    target: u64,
) -> Option<u64> {
    let offset = target_in_window_coordinates(latest, window.anchor, target)?;
    window.visible_rows(offset)?;
    // Historical cells are immutable within an epoch. Cells captured from the
    // mutable live screen cannot become history merely by translating indices.
    if latest.revision != window.anchor.revision && offset < u64::from(window.anchor.viewport.rows)
    {
        return None;
    }
    Some(offset)
}
