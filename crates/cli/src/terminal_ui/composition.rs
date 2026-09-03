use std::collections::BTreeMap;

use unicode_width::UnicodeWidthChar;
use zterm_core::ResourceLimits;
use zterm_core::terminal::{
    ActiveScreen, TerminalCell, TerminalModes, TerminalScrollMetrics, TerminalSize, TerminalStyle,
    TerminalSurface,
};
use zterm_daemon::operations::TerminalViewTransportState;

use super::{CliError, StatusRenderer, ViewportController};

/// Non-overlapping allocation of child content and host-owned chrome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ChromeLayout {
    pub(super) child: TerminalSize,
    pub(super) gutter_column: Option<u16>,
    pub(super) status_row: Option<u16>,
}

impl ChromeLayout {
    pub(super) fn new(physical: TerminalSize, remote: bool, screen: ActiveScreen) -> Self {
        let limits = ResourceLimits::default();
        let status_row = (remote && physical.rows > 1).then_some(physical.rows);
        let status_rows = u16::from(status_row.is_some());
        let usable_rows = physical
            .rows
            .saturating_sub(status_rows)
            .min(limits.max_viewport_rows)
            .max(1);
        let usable_columns = physical.columns.min(limits.max_viewport_columns).max(1);
        let gutter_column =
            (screen == ActiveScreen::Main && usable_columns > 4).then_some(usable_columns);
        let child_columns = usable_columns.saturating_sub(u16::from(gutter_column.is_some()));
        Self {
            child: TerminalSize::new(usable_rows, child_columns.max(1)),
            gutter_column,
            status_row,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ScrollbarGeometry {
    track_rows: u16,
    pub(super) thumb_top: u16,
    pub(super) thumb_len: u16,
}

impl ScrollbarGeometry {
    pub(super) fn new(track_rows: u16, metrics: TerminalScrollMetrics) -> Option<Self> {
        if track_rows == 0 || !metrics.is_valid() || metrics.max_offset_from_bottom == 0 {
            return None;
        }
        let track = u128::from(track_rows);
        let visible = u128::from(metrics.viewport_rows);
        let maximum = u128::from(metrics.max_offset_from_bottom);
        let thumb_len =
            ((track.checked_mul(visible)? / visible.checked_add(maximum)?).max(1)).min(track);
        let travel = track.saturating_sub(thumb_len);
        let newer_distance = maximum.saturating_sub(u128::from(
            metrics
                .offset_from_bottom
                .min(metrics.max_offset_from_bottom),
        ));
        let thumb_top = if travel == 0 {
            0
        } else {
            newer_distance
                .checked_mul(travel)?
                .checked_add(maximum / 2)?
                / maximum
        };
        Some(Self {
            track_rows,
            thumb_top: u16::try_from(thumb_top.min(track)).ok()?,
            thumb_len: u16::try_from(thumb_len).ok()?,
        })
    }

    pub(super) fn contains_thumb(self, row: u16) -> bool {
        let row = row.saturating_sub(1);
        row >= self.thumb_top && row < self.thumb_top.saturating_add(self.thumb_len)
    }

    pub(super) fn grab_row(self, row: u16) -> u16 {
        row.saturating_sub(1)
            .saturating_sub(self.thumb_top)
            .min(self.thumb_len.saturating_sub(1))
    }

    pub(super) fn offset_for_pointer(
        self,
        row: u16,
        grab_row: u16,
        maximum_offset: u64,
    ) -> u64 {
        if maximum_offset == 0 {
            return 0;
        }
        let travel = self.track_rows.saturating_sub(self.thumb_len);
        if travel == 0 {
            return maximum_offset;
        }
        let pointer = row.saturating_sub(1).min(self.track_rows.saturating_sub(1));
        let top = pointer
            .saturating_sub(grab_row.min(self.thumb_len.saturating_sub(1)))
            .min(travel);
        let maximum = u128::from(maximum_offset);
        let newer_distance = u128::from(top)
            .saturating_mul(maximum)
            .saturating_add(u128::from(travel) / 2)
            / u128::from(travel);
        maximum_offset.saturating_sub(
            u64::try_from(newer_distance.min(maximum)).unwrap_or(maximum_offset),
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ComposedCursor {
    pub(super) row: u16,
    pub(super) column: u16,
    pub(super) visible: bool,
    pub(super) style: TerminalStyle,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LayoutIdentity {
    pub(super) content_size: TerminalSize,
    pub(super) gutter_column: Option<u16>,
    pub(super) status_row: Option<u16>,
}

/// Complete renderer-neutral desired frame for one physical transition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ComposedFrame {
    pub(super) physical_size: TerminalSize,
    pub(super) layout: LayoutIdentity,
    pub(super) rows: BTreeMap<u16, Vec<TerminalCell>>,
    pub(super) cursor: ComposedCursor,
    pub(super) modes: TerminalModes,
}

impl ComposedFrame {
    pub(super) fn compose(
        surface: &TerminalSurface,
        previous: Option<&Self>,
        viewport: &ViewportController,
        status: &StatusRenderer,
        transport_state: TerminalViewTransportState,
    ) -> Result<Self, CliError> {
        let physical_size = status.physical_size;
        let mut rows = BTreeMap::new();
        let height = usize::from(viewport.content_size.rows);
        let width = usize::from(viewport.content_size.columns);
        let semantic_history = viewport.visible_semantic_history_rows();
        let history_source = semantic_history.as_ref().map(|(rows, _, _)| rows);
        let history_notice = semantic_history.as_ref().and_then(|(_, _, notice)| *notice);

        for row_index in 0..height {
            let mut row = if viewport.is_live() {
                surface
                    .rows
                    .get(row_index)
                    .map(|row| row.cells.clone())
                    .unwrap_or_default()
            } else if let Some(history) = history_source {
                let padding = height.saturating_sub(history.len());
                row_index
                    .checked_sub(padding)
                    .and_then(|index| history.get(index))
                    .map(|row| row.cells.clone())
                    .unwrap_or_default()
            } else {
                previous
                    .and_then(|frame| {
                        frame
                            .rows
                            .get(&u16::try_from(row_index).unwrap_or(u16::MAX))
                    })
                    .map(|row| row[..row.len().min(width)].to_vec())
                    .unwrap_or_default()
            };
            normalize_composed_row(&mut row, width);
            if row_index == 0
                && let Some(notice) = history_notice
            {
                row = text_cells(notice, width, TerminalStyle::default());
            }
            if let Some(column) = viewport.gutter_column {
                let geometry = viewport
                    .scroll_metrics()
                    .and_then(|metrics| ScrollbarGeometry::new(viewport.content_size.rows, metrics));
                let glyph = match geometry {
                    Some(geometry)
                        if u16::try_from(row_index).is_ok_and(|row| {
                            row >= geometry.thumb_top
                                && row < geometry.thumb_top.saturating_add(geometry.thumb_len)
                        }) =>
                    {
                        "▐"
                    }
                    Some(_) => "▕",
                    None => " ",
                };
                let gutter_index = usize::from(column.saturating_sub(1));
                if row.len() <= gutter_index {
                    row.resize(gutter_index + 1, TerminalCell::default());
                }
                row[gutter_index] = TerminalCell {
                    contents: glyph.to_owned(),
                    ..TerminalCell::default()
                };
            }
            rows.insert(u16::try_from(row_index).unwrap_or(u16::MAX), row);
        }

        if let (Some(status_row), Some(text)) = (
            (status.enabled()).then_some(physical_size.rows.saturating_sub(1)),
            status.composed_text(transport_state),
        ) {
            rows.insert(
                status_row,
                text_cells(
                    &text,
                    usize::from(
                        physical_size
                            .columns
                            .min(ResourceLimits::default().max_viewport_columns),
                    ),
                    TerminalStyle {
                        inverse: true,
                        ..TerminalStyle::default()
                    },
                ),
            );
        }

        let cursor = if transport_state == TerminalViewTransportState::Active
            && viewport.is_live()
            && surface.cursor.visible
            && surface.cursor.row < viewport.content_size.rows
            && surface.cursor.column < viewport.content_size.columns
        {
            ComposedCursor {
                row: surface.cursor.row,
                column: surface.cursor.column,
                visible: true,
                style: surface.cursor.style,
            }
        } else {
            ComposedCursor {
                row: 0,
                column: 0,
                visible: false,
                style: TerminalStyle::default(),
            }
        };
        Ok(Self {
            physical_size,
            layout: LayoutIdentity {
                content_size: viewport.content_size,
                gutter_column: viewport.gutter_column,
                status_row: status
                    .enabled()
                    .then_some(physical_size.rows.saturating_sub(1)),
            },
            rows,
            cursor,
            modes: surface.modes,
        })
    }
}

pub(super) fn normalize_composed_row(row: &mut Vec<TerminalCell>, width: usize) {
    row.truncate(width);
    row.resize(width, TerminalCell::default());
    if row.last().is_some_and(|cell| cell.wide) {
        *row.last_mut().expect("row has a final cell") = TerminalCell::default();
    }
    for index in 0..row.len() {
        if row[index].wide
            && row
                .get(index + 1)
                .is_none_or(|cell| !cell.wide_continuation)
        {
            row[index] = TerminalCell::default();
        }
        if row[index].wide_continuation && (index == 0 || !row[index - 1].wide) {
            row[index] = TerminalCell::default();
        }
    }
}

fn text_cells(text: &str, width: usize, style: TerminalStyle) -> Vec<TerminalCell> {
    let mut cells = Vec::with_capacity(width);
    for character in text.chars() {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if character_width == 0 {
            if let Some(previous) = cells
                .iter_mut()
                .rev()
                .find(|cell: &&mut TerminalCell| !cell.wide_continuation)
            {
                previous.contents.push(character);
            }
            continue;
        }
        if character_width > 2 || cells.len().saturating_add(character_width) > width {
            break;
        }
        cells.push(TerminalCell {
            contents: character.to_string(),
            wide: character_width == 2,
            wide_continuation: false,
            style,
        });
        if character_width == 2 {
            cells.push(TerminalCell {
                wide_continuation: true,
                style,
                ..TerminalCell::default()
            });
        }
    }
    cells.resize(
        width,
        TerminalCell {
            style,
            ..TerminalCell::default()
        },
    );
    cells
}
