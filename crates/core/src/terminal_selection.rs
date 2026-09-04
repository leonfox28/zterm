//! Renderer-neutral terminal text-range normalization and extraction.

use std::fmt;

use crate::terminal::{
    MAX_TERMINAL_CLIPBOARD_BYTES, TerminalCell, TerminalClipboardError, TerminalClipboardWrite,
    TerminalSurfaceRow,
};

/// Zero-based semantic terminal cell coordinate.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TerminalTextPoint {
    /// Visible row index.
    pub row: u16,
    /// Visible column index.
    pub column: u16,
}

impl TerminalTextPoint {
    /// Creates one zero-based terminal cell coordinate.
    #[must_use]
    pub const fn new(row: u16, column: u16) -> Self {
        Self { row, column }
    }
}

/// Inclusive reading-order terminal cell range.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalTextRange {
    /// First selected cell in reading order.
    pub start: TerminalTextPoint,
    /// Final selected cell in reading order.
    pub end: TerminalTextPoint,
}

impl TerminalTextRange {
    /// Normalizes either drag direction into one inclusive reading-order range.
    #[must_use]
    pub fn new(anchor: TerminalTextPoint, focus: TerminalTextPoint) -> Self {
        let (start, end) = if anchor <= focus {
            (anchor, focus)
        } else {
            (focus, anchor)
        };
        Self { start, end }
    }

    /// Returns whether this normalized range includes one cell.
    #[must_use]
    pub fn contains(self, point: TerminalTextPoint) -> bool {
        self.start <= point && point <= self.end
    }

    /// Expands endpoints so a wide glyph is selected atomically.
    pub fn expand_wide(
        self,
        rows: &[TerminalSurfaceRow],
    ) -> Result<Self, TerminalTextSelectionError> {
        validate_point(rows, self.start)?;
        validate_point(rows, self.end)?;
        let mut expanded = self;

        let start = cell(rows, expanded.start)?;
        if start.wide_continuation {
            let Some(column) = expanded.start.column.checked_sub(1) else {
                return Err(TerminalTextSelectionError::InvalidSurface);
            };
            let previous = TerminalTextPoint::new(expanded.start.row, column);
            if !cell(rows, previous)?.wide {
                return Err(TerminalTextSelectionError::InvalidSurface);
            }
            expanded.start = previous;
        }

        let end = cell(rows, expanded.end)?;
        if end.wide {
            let Some(column) = expanded.end.column.checked_add(1) else {
                return Err(TerminalTextSelectionError::InvalidSurface);
            };
            let continuation = TerminalTextPoint::new(expanded.end.row, column);
            if !cell(rows, continuation)?.wide_continuation {
                return Err(TerminalTextSelectionError::InvalidSurface);
            }
            expanded.end = continuation;
        }
        Ok(expanded)
    }

    /// Extracts exact semantic text under the shared clipboard byte limit.
    pub fn extract(
        self,
        rows: &[TerminalSurfaceRow],
    ) -> Result<TerminalClipboardWrite, TerminalTextSelectionError> {
        let range = self.expand_wide(rows)?;
        let mut text = String::new();

        for row_index in range.start.row..=range.end.row {
            let row = rows
                .get(usize::from(row_index))
                .ok_or(TerminalTextSelectionError::InvalidRange)?;
            let first_column = if row_index == range.start.row {
                range.start.column
            } else {
                0
            };
            let last_column = if row_index == range.end.row {
                range.end.column
            } else {
                u16::try_from(row.cells.len().saturating_sub(1))
                    .map_err(|_| TerminalTextSelectionError::InvalidSurface)?
            };

            for column in first_column..=last_column {
                let selected = row
                    .cells
                    .get(usize::from(column))
                    .ok_or(TerminalTextSelectionError::InvalidRange)?;
                if selected.wide_continuation {
                    continue;
                }
                let contents = if selected.contents.is_empty() {
                    " "
                } else {
                    selected.contents.as_str()
                };
                append_bounded(&mut text, contents)?;
            }

            if row_index < range.end.row && !row.wrapped {
                append_bounded(&mut text, "\n")?;
            }
        }

        TerminalClipboardWrite::new(text).map_err(TerminalTextSelectionError::Clipboard)
    }
}

/// Failure to normalize or extract a semantic terminal selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalTextSelectionError {
    /// A range endpoint lies outside the supplied rows.
    InvalidRange,
    /// Supplied rows violate the validated semantic-cell assumptions.
    InvalidSurface,
    /// Extracted text violates the shared clipboard contract.
    Clipboard(TerminalClipboardError),
}

impl fmt::Display for TerminalTextSelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidRange => "terminal text selection is outside the visible rows",
            Self::InvalidSurface => "terminal text selection source is structurally invalid",
            Self::Clipboard(error) => return error.fmt(formatter),
        })
    }
}

impl std::error::Error for TerminalTextSelectionError {}

fn validate_point(
    rows: &[TerminalSurfaceRow],
    point: TerminalTextPoint,
) -> Result<(), TerminalTextSelectionError> {
    let row = rows
        .get(usize::from(point.row))
        .ok_or(TerminalTextSelectionError::InvalidRange)?;
    if usize::from(point.column) >= row.cells.len() {
        return Err(TerminalTextSelectionError::InvalidRange);
    }
    Ok(())
}

fn cell(
    rows: &[TerminalSurfaceRow],
    point: TerminalTextPoint,
) -> Result<&TerminalCell, TerminalTextSelectionError> {
    rows.get(usize::from(point.row))
        .and_then(|row| row.cells.get(usize::from(point.column)))
        .ok_or(TerminalTextSelectionError::InvalidRange)
}

fn append_bounded(destination: &mut String, value: &str) -> Result<(), TerminalTextSelectionError> {
    if destination
        .len()
        .checked_add(value.len())
        .is_none_or(|length| length > MAX_TERMINAL_CLIPBOARD_BYTES)
    {
        return Err(TerminalTextSelectionError::Clipboard(
            TerminalClipboardError::TooLarge,
        ));
    }
    destination.push_str(value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell_with(contents: &str) -> TerminalCell {
        TerminalCell {
            contents: contents.to_owned(),
            ..TerminalCell::default()
        }
    }

    #[test]
    fn extraction_normalizes_direction_blanks_and_wrapped_rows() {
        let rows = vec![
            TerminalSurfaceRow {
                cells: vec![cell_with("a"), TerminalCell::default(), cell_with("b")],
                wrapped: true,
            },
            TerminalSurfaceRow {
                cells: vec![cell_with("c"), cell_with("d"), cell_with("e")],
                wrapped: false,
            },
            TerminalSurfaceRow {
                cells: vec![cell_with("f"), cell_with("g"), cell_with("h")],
                wrapped: false,
            },
        ];
        let reverse =
            TerminalTextRange::new(TerminalTextPoint::new(2, 1), TerminalTextPoint::new(0, 1));
        assert_eq!(
            reverse.extract(&rows).expect("selection").as_str(),
            " bcde\nfg"
        );
    }

    #[test]
    fn wide_endpoints_expand_and_combining_text_is_preserved_once() {
        let rows = vec![TerminalSurfaceRow {
            cells: vec![
                cell_with("x"),
                TerminalCell {
                    contents: "界\u{301}".to_owned(),
                    wide: true,
                    ..TerminalCell::default()
                },
                TerminalCell {
                    wide_continuation: true,
                    ..TerminalCell::default()
                },
                cell_with("y"),
            ],
            wrapped: false,
        }];
        for column in [1, 2] {
            let range = TerminalTextRange::new(
                TerminalTextPoint::new(0, column),
                TerminalTextPoint::new(0, column),
            );
            let expanded = range.expand_wide(&rows).expect("wide range");
            assert_eq!(expanded.start.column, 1);
            assert_eq!(expanded.end.column, 2);
            assert_eq!(
                range.extract(&rows).expect("wide text").as_str(),
                "界\u{301}"
            );
        }
    }

    #[test]
    fn extraction_enforces_the_clipboard_cap_atomically() {
        let mut cells = vec![cell_with("abcdefghijklmnopqrstuv"); 23_831];
        cells.push(cell_with("abcdef"));
        let exact = vec![TerminalSurfaceRow {
            cells: cells.clone(),
            wrapped: false,
        }];
        let range = TerminalTextRange::new(
            TerminalTextPoint::new(0, 0),
            TerminalTextPoint::new(0, 23_831),
        );
        assert_eq!(
            range.extract(&exact).expect("exact cap").as_str().len(),
            524_288
        );

        cells.last_mut().expect("last cell").contents.push('g');
        let over = vec![TerminalSurfaceRow {
            cells,
            wrapped: false,
        }];
        assert_eq!(
            range.extract(&over),
            Err(TerminalTextSelectionError::Clipboard(
                TerminalClipboardError::TooLarge
            ))
        );
    }
}
