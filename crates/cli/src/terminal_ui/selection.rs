use zterm_core::Revision;
use zterm_core::terminal::{
    ActiveScreen, TerminalClipboardWrite, TerminalSize, TerminalSurfaceRow,
};
use zterm_core::terminal_selection::{
    TerminalTextPoint, TerminalTextRange, TerminalTextSelectionError,
};
use zterm_core::viewport_cache::ViewportSliceIdentity;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SelectionSourceIdentity {
    Live {
        revision: Revision,
        screen: ActiveScreen,
        viewport: TerminalSize,
    },
    History(ViewportSliceIdentity),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SelectionState {
    Idle,
    Dragging {
        source: SelectionSourceIdentity,
        anchor: TerminalTextPoint,
        range: TerminalTextRange,
        moved: bool,
    },
    Finalized {
        source: SelectionSourceIdentity,
        range: TerminalTextRange,
    },
    CancelledUntilRelease,
}

/// Compact selection state supplied to one presenter transaction.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct SelectionPresentation {
    source: Option<SelectionSourceIdentity>,
    range: Option<TerminalTextRange>,
    copy_ready: bool,
}

impl SelectionPresentation {
    pub(super) fn from_parts(
        source: Option<SelectionSourceIdentity>,
        range: Option<TerminalTextRange>,
        copy_ready: bool,
    ) -> Self {
        let range = source.and(range);
        Self {
            source: source.filter(|_| range.is_some()),
            range,
            copy_ready: source.is_some() && copy_ready && range.is_some(),
        }
    }

    pub(super) fn range_for(
        self,
        source: Option<SelectionSourceIdentity>,
    ) -> Option<TerminalTextRange> {
        if source.is_some() && self.source == source {
            self.range
        } else {
            None
        }
    }

    pub(super) fn copy_ready_for(
        self,
        source: Option<SelectionSourceIdentity>,
    ) -> bool {
        source.is_some() && self.source == source && self.copy_ready
    }

    pub(super) const fn copy_ready(self) -> bool {
        self.copy_ready
    }
}

/// Attachment-local selection state with no renderer, transport, or clipboard
/// backend ownership.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct SelectionController {
    state: SelectionState,
}

impl Default for SelectionController {
    fn default() -> Self {
        Self {
            state: SelectionState::Idle,
        }
    }
}

impl SelectionController {
    pub(super) const fn owns_pointer_sequence(&self) -> bool {
        matches!(
            self.state,
            SelectionState::Dragging { .. } | SelectionState::CancelledUntilRelease
        )
    }

    pub(super) const fn is_finalized(&self) -> bool {
        matches!(self.state, SelectionState::Finalized { .. })
    }

    pub(super) fn begin(
        &mut self,
        source: SelectionSourceIdentity,
        point: TerminalTextPoint,
        rows: &[TerminalSurfaceRow],
    ) -> Result<(), TerminalTextSelectionError> {
        let range = TerminalTextRange::new(point, point).expand_wide(rows)?;
        self.state = SelectionState::Dragging {
            source,
            anchor: point,
            range,
            moved: false,
        };
        Ok(())
    }

    pub(super) fn update(
        &mut self,
        source: SelectionSourceIdentity,
        point: TerminalTextPoint,
        rows: &[TerminalSurfaceRow],
    ) -> Result<(), TerminalTextSelectionError> {
        let SelectionState::Dragging {
            source: selected,
            anchor,
            moved,
            ..
        } = self.state
        else {
            return Ok(());
        };
        if selected != source {
            self.state = SelectionState::CancelledUntilRelease;
            return Ok(());
        }
        let range = TerminalTextRange::new(anchor, point).expand_wide(rows)?;
        self.state = SelectionState::Dragging {
            source,
            anchor,
            range,
            moved: moved || point != anchor,
        };
        Ok(())
    }

    pub(super) fn finish(&mut self) {
        self.state = match self.state {
            SelectionState::Dragging {
                source,
                range,
                moved: true,
                ..
            } => SelectionState::Finalized { source, range },
            SelectionState::Dragging { .. } | SelectionState::CancelledUntilRelease => {
                SelectionState::Idle
            }
            current => current,
        };
    }

    pub(super) fn reconcile(&mut self, source: Option<SelectionSourceIdentity>) {
        let selected = match self.state {
            SelectionState::Dragging { source, .. }
            | SelectionState::Finalized { source, .. } => source,
            SelectionState::Idle | SelectionState::CancelledUntilRelease => return,
        };
        if Some(selected) != source {
            self.state = if matches!(self.state, SelectionState::Dragging { .. }) {
                SelectionState::CancelledUntilRelease
            } else {
                SelectionState::Idle
            };
        }
    }

    pub(super) fn presentation(
        &self,
        source: Option<SelectionSourceIdentity>,
    ) -> SelectionPresentation {
        SelectionPresentation::from_parts(
            source,
            self.range_for(source),
            self.is_finalized(),
        )
    }

    pub(super) fn cancel(&mut self) {
        self.state = if matches!(self.state, SelectionState::Dragging { .. }) {
            SelectionState::CancelledUntilRelease
        } else {
            SelectionState::Idle
        };
    }

    pub(super) fn clear(&mut self) {
        self.state = SelectionState::Idle;
    }

    pub(super) fn range_for(
        &self,
        source: Option<SelectionSourceIdentity>,
    ) -> Option<TerminalTextRange> {
        match self.state {
            SelectionState::Dragging {
                source: selected,
                range,
                ..
            }
            | SelectionState::Finalized {
                source: selected,
                range,
            } if Some(selected) == source => Some(range),
            _ => None,
        }
    }

    pub(super) fn extract(
        &self,
        source: SelectionSourceIdentity,
        rows: &[TerminalSurfaceRow],
    ) -> Result<Option<TerminalClipboardWrite>, TerminalTextSelectionError> {
        let SelectionState::Finalized {
            source: selected,
            range,
        } = self.state
        else {
            return Ok(None);
        };
        if selected != source {
            return Ok(None);
        }
        range.extract(rows).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use zterm_core::terminal::TerminalCell;

    use super::*;

    fn source(revision: u64) -> SelectionSourceIdentity {
        SelectionSourceIdentity::Live {
            revision: Revision::new(revision),
            screen: ActiveScreen::Main,
            viewport: TerminalSize::new(1, 3),
        }
    }

    fn rows() -> Vec<TerminalSurfaceRow> {
        vec![TerminalSurfaceRow {
            cells: ["a", "b", "c"]
                .into_iter()
                .map(|contents| TerminalCell {
                    contents: contents.to_owned(),
                    ..TerminalCell::default()
                })
                .collect(),
            wrapped: false,
        }]
    }

    #[test]
    fn click_is_empty_but_drag_finalizes_and_extracts() {
        let rows = rows();
        let mut selection = SelectionController::default();
        selection
            .begin(source(1), TerminalTextPoint::new(0, 1), &rows)
            .expect("valid click start");
        selection.finish();
        assert!(!selection.is_finalized());

        selection
            .begin(source(1), TerminalTextPoint::new(0, 2), &rows)
            .expect("valid reverse drag start");
        selection
            .update(source(1), TerminalTextPoint::new(0, 0), &rows)
            .expect("valid reverse drag update");
        selection.finish();
        assert!(selection.is_finalized());
        assert_eq!(
            selection
                .extract(source(1), &rows)
                .expect("selection source matches")
                .expect("finalized selection extracts")
                .as_str(),
            "abc"
        );
    }

    #[test]
    fn source_change_during_drag_keeps_capture_until_release() {
        let rows = rows();
        let mut selection = SelectionController::default();
        selection
            .begin(source(1), TerminalTextPoint::new(0, 0), &rows)
            .expect("valid drag start");
        selection.reconcile(Some(source(2)));
        assert!(selection.owns_pointer_sequence());
        assert!(selection.range_for(Some(source(2))).is_none());
        selection.finish();
        assert!(!selection.owns_pointer_sequence());
        assert!(!selection.is_finalized());
    }
}
