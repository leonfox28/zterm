//! Self-contained, deduplicated link dictionaries for semantic messages.
use crate::{ProtocolError, v2};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use zterm_core::terminal::{
    MAX_TERMINAL_HYPERLINK_BYTES, MAX_TERMINAL_HYPERLINKS, TerminalCell, TerminalHyperlink,
    TerminalSurfaceRow,
};

#[derive(Default)]
pub(crate) struct LinkEncoder {
    indices: HashMap<Arc<TerminalHyperlink>, u32>,
    values: Vec<v2::TerminalHyperlink>,
}

impl LinkEncoder {
    pub(crate) fn row(&mut self, row: TerminalSurfaceRow) -> v2::TerminalSurfaceRow {
        v2::TerminalSurfaceRow {
            wrapped: row.wrapped,
            cells: row
                .cells
                .into_iter()
                .map(|cell| {
                    let hyperlink = cell.hyperlink.map_or(0, |link| {
                        if let Some(index) = self.indices.get(&link) {
                            return *index;
                        }
                        let index = u32::try_from(self.values.len() + 1).unwrap_or(u32::MAX);
                        self.values.push(v2::TerminalHyperlink {
                            id: link.id().to_owned(),
                            uri: link.uri().to_owned(),
                        });
                        self.indices.insert(link, index);
                        index
                    });
                    v2::TerminalCell {
                        hyperlink,
                        contents: cell.contents,
                        wide: cell.wide,
                        wide_continuation: cell.wide_continuation,
                        style: Some(cell.style.into()),
                    }
                })
                .collect(),
        }
    }
    pub(crate) fn finish(self) -> Vec<v2::TerminalHyperlink> {
        self.values
    }
}

pub(crate) fn decode_links(
    values: Vec<v2::TerminalHyperlink>,
) -> Result<Vec<Arc<TerminalHyperlink>>, ProtocolError> {
    let invalid = || ProtocolError::InvalidTerminalSemanticField("hyperlinks");
    if values.len() > MAX_TERMINAL_HYPERLINKS {
        return Err(invalid());
    }
    let mut bytes = 0usize;
    let mut unique = HashSet::new();
    values
        .into_iter()
        .map(|value| {
            let link =
                Arc::new(TerminalHyperlink::new(&value.id, &value.uri).map_err(|_| invalid())?);
            bytes = bytes.saturating_add(link.payload_bytes());
            if bytes > MAX_TERMINAL_HYPERLINK_BYTES || !unique.insert(Arc::clone(&link)) {
                return Err(invalid());
            }
            Ok(link)
        })
        .collect()
}

pub(crate) fn decode_row(
    value: v2::TerminalSurfaceRow,
    links: &[Arc<TerminalHyperlink>],
) -> Result<TerminalSurfaceRow, ProtocolError> {
    Ok(TerminalSurfaceRow {
        wrapped: value.wrapped,
        cells: value
            .cells
            .into_iter()
            .map(|cell| {
                let hyperlink = if cell.hyperlink == 0 {
                    None
                } else {
                    Some(Arc::clone(links.get(cell.hyperlink as usize - 1).ok_or(
                        ProtocolError::InvalidTerminalSemanticField("hyperlink_reference"),
                    )?))
                };
                Ok(TerminalCell {
                    hyperlink,
                    contents: cell.contents,
                    wide: cell.wide,
                    wide_continuation: cell.wide_continuation,
                    style: cell
                        .style
                        .ok_or(ProtocolError::InvalidTerminalSemanticField("cell_style"))?
                        .try_into()?,
                })
            })
            .collect::<Result<_, ProtocolError>>()?,
    })
}

impl std::fmt::Debug for v2::TerminalHyperlink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TerminalHyperlink([REDACTED])")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn long_targets_are_encoded_once_and_decoded_cells_share_the_allocation() {
        let link = Arc::new(
            TerminalHyperlink::new("id", &format!("https://example.com/{}", "x".repeat(900)))
                .expect("link"),
        );
        let row = TerminalSurfaceRow {
            wrapped: false,
            cells: vec![
                TerminalCell {
                    hyperlink: Some(link),
                    contents: "x".into(),
                    ..Default::default()
                };
                240
            ],
        };
        let mut encoder = LinkEncoder::default();
        let encoded = encoder.row(row.clone());
        let dictionary = encoder.finish();
        assert_eq!(dictionary.len(), 1);
        assert!(encoded.cells.iter().all(|c| c.hyperlink == 1));
        assert!(!format!("{:?}", dictionary[0]).contains("example.com"));
        let links = decode_links(dictionary.clone()).expect("dictionary");
        let decoded = decode_row(encoded.clone(), &links).expect("row");
        assert_eq!(decoded, row);
        assert!(Arc::ptr_eq(
            decoded.cells[0].hyperlink.as_ref().expect("link"),
            decoded.cells[239].hyperlink.as_ref().expect("shared")
        ));
        assert!(decode_links([dictionary.clone(), dictionary].concat()).is_err());
        let mut invalid = encoded;
        invalid.cells[0].hyperlink = 2;
        assert!(decode_row(invalid, &links).is_err());
        assert!(
            decode_links(vec![v2::TerminalHyperlink {
                id: "x".into(),
                uri: "file:///remote".into()
            }])
            .is_err()
        );
    }
}
