//! Bounded snapshot readers shared by tail and export.

use crate::{Level, MAX_RECORD_BYTES, Record, now_ms};
use serde::Serialize;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

/// Total input budget for one tail request, including both lanes.
pub const TAIL_BYTES: u64 = 1024 * 1024;
/// Hard tail record limit.
pub const TAIL_LINES: usize = 1000;
/// Validated file handle and length captured before releasing writer coordination.
pub struct Snapshot {
    /// Open read handle retained across rename/rotation.
    pub file: File,
    /// Snapshot length, capped by the storage owner.
    pub length: u64,
    /// Skipped oversized legacy prefix.
    pub start: u64,
    /// Detail lane, independent of key retention.
    pub detail: bool,
}
/// One-shot tail query. Filtering searches only the bounded input tail.
#[derive(Default)]
pub struct Filter {
    /// Maximum returned records (also capped by TAIL_LINES).
    pub lines: usize,
    /// Minimum severity.
    pub level: Option<Level>,
    /// Exact fixed component.
    pub component: Option<String>,
    /// Exact canonical Session ID.
    pub session: Option<String>,
    /// Earliest UTC Unix milliseconds.
    pub since_ms: Option<i64>,
    /// Include the independently bounded detail lane.
    pub include_debug: bool,
    /// Accept only structured records.
    pub json: bool,
}
impl Filter {
    fn structured(&self) -> bool {
        self.json
            || self.level.is_some()
            || self.component.is_some()
            || self.session.is_some()
            || self.since_ms.is_some()
    }
    fn matches(&self, record: &Record) -> bool {
        self.level.is_none_or(|level| record.level >= level)
            && self
                .component
                .as_ref()
                .is_none_or(|value| *value == record.component)
            && self
                .session
                .as_ref()
                .is_none_or(|value| Some(value) == record.fields.session_id.as_ref())
            && self.since_ms.is_none_or(|since| {
                OffsetDateTime::parse(&record.timestamp, &Rfc3339)
                    .is_ok_and(|time| time.unix_timestamp_nanos() / 1_000_000 >= i128::from(since))
            })
    }
}
/// Bounded inspection result plus visible omission accounting.
#[derive(Default)]
pub struct Tail {
    /// Already rendered human lines or validated JSON records.
    pub lines: Vec<String>,
    /// Legacy/invalid/oversized/partial records omitted from structured output.
    pub omitted: u64,
    /// Whether older bytes were outside the tail budget.
    pub truncated: bool,
}

/// Reads snapshots supplied in newest-file-first order within each lane.
pub fn tail(snapshots: &mut [Snapshot], filter: &Filter) -> io::Result<Tail> {
    let count = filter.lines.min(TAIL_LINES);
    if count == 0 {
        return Ok(Tail::default());
    }
    let detail = filter.include_debug || filter.level == Some(Level::Debug);
    let budget = if detail { TAIL_BYTES / 2 } else { TAIL_BYTES };
    let mut remaining = [budget, if detail { budget } else { 0 }];
    let mut output = Tail::default();
    let mut records = Vec::new();
    // Reverse later so archives precede the current file when legacy timestamps
    // are unavailable. Structured records are merged by timestamp below.
    for (file_order, snapshot) in snapshots.iter_mut().enumerate() {
        let lane = usize::from(snapshot.detail);
        let limit = snapshot.length.min(remaining[lane]);
        remaining[lane] -= limit;
        if snapshot.start > 0 || snapshot.length > limit {
            output.truncated = true;
        }
        if limit == 0 {
            continue;
        }
        let start = snapshot.start + snapshot.length - limit;
        snapshot.file.seek(SeekFrom::Start(start))?;
        let mut invalid = 0;
        scan(
            (&mut snapshot.file).take(limit),
            start > 0,
            |bytes| {
                if let Some(record) = Record::decode(bytes) {
                    if filter.matches(&record) {
                        let line = if filter.json {
                            String::from_utf8(record.encode().unwrap_or_default())
                                .unwrap_or_default()
                                .trim_end()
                                .to_owned()
                        } else {
                            record.render()
                        };
                        records.push((
                            OffsetDateTime::parse(&record.timestamp, &Rfc3339)
                                .map_or(i128::MIN, |time| time.unix_timestamp_nanos()),
                            record.process_instance,
                            record.sequence,
                            usize::MAX - file_order,
                            line,
                        ));
                    }
                } else if !filter.structured() {
                    let text = String::from_utf8_lossy(bytes)
                        .chars()
                        .filter(|c| !c.is_control() || *c == '\t')
                        .collect::<String>();
                    records.push((i128::MIN, String::new(), 0, usize::MAX - file_order, text));
                } else {
                    invalid += 1;
                }
            },
            &mut output.omitted,
        )?;
        output.omitted += invalid;
    }
    if detail {
        records.sort_by(|a, b| (&a.0, &a.1, a.2, a.3).cmp(&(&b.0, &b.1, b.2, b.3)));
    } else {
        records.sort_by_key(|record| record.3);
    }
    output.lines = records
        .into_iter()
        .rev()
        .take(count)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|record| record.4)
        .collect();
    Ok(output)
}

/// Export metadata contains no paths, host identities or configuration values.
#[derive(Serialize)]
pub struct ExportHeader {
    /// Distinguishes the header from event records.
    pub export_schema: u8,
    /// Exporting build.
    pub version: &'static str,
    /// Compile-time platform.
    pub platform: &'static str,
    /// Whether detailed records were requested.
    pub includes_debug: bool,
    /// Invalid or legacy records excluded from the export.
    pub omitted: u64,
    /// Bytes not scanned because of the retained-file bounds.
    pub truncated_bytes: u64,
    /// Recorder's currently unreported loss count; historical summaries stay in events.
    pub pending_lost: u64,
}
impl ExportHeader {
    /// Constructs the bounded safe header used by both exporters.
    pub fn new(
        includes_debug: bool,
        omitted: u64,
        truncated_bytes: u64,
        pending_lost: u64,
    ) -> Self {
        Self {
            export_schema: 1,
            version: env!("CARGO_PKG_VERSION"),
            platform: std::env::consts::OS,
            includes_debug,
            omitted,
            truncated_bytes,
            pending_lost,
        }
    }
    /// One complete JSONL header.
    pub fn encode(&self) -> io::Result<Vec<u8>> {
        let mut bytes = serde_json::to_vec(self)?;
        bytes.push(b'\n');
        Ok(bytes)
    }
}
/// Streams all recognized records from the selected bounded snapshots.
pub fn export(
    snapshots: &mut [Snapshot],
    output: &mut dyn Write,
    include_debug: bool,
    pending_lost: u64,
) -> io::Result<ExportHeader> {
    let mut omitted = 0;
    // The first bounded pass counts omissions without buffering the history.
    for snapshot in snapshots.iter_mut().filter(|s| include_debug || !s.detail) {
        let mut invalid = 0;
        snapshot.file.seek(SeekFrom::Start(snapshot.start))?;
        scan(
            (&mut snapshot.file).take(snapshot.length),
            snapshot.start > 0,
            |bytes| {
                if Record::decode(bytes).is_none() {
                    invalid += 1;
                }
            },
            &mut omitted,
        )?;
        omitted += invalid;
    }
    let header = ExportHeader::new(
        include_debug,
        omitted,
        snapshots.iter().map(|s| s.start).sum(),
        pending_lost,
    );
    output.write_all(&header.encode()?)?;
    for snapshot in snapshots
        .iter_mut()
        .rev()
        .filter(|s| include_debug || !s.detail)
    {
        snapshot.file.seek(SeekFrom::Start(snapshot.start))?;
        let mut write_error = None;
        scan(
            (&mut snapshot.file).take(snapshot.length),
            snapshot.start > 0,
            |bytes| {
                if let Some(record) = Record::decode(bytes)
                    && let Some(bytes) = record.encode()
                    && write_error.is_none()
                {
                    write_error = output.write_all(&bytes).err();
                }
            },
            &mut 0,
        )?;
        if let Some(error) = write_error {
            return Err(error);
        }
    }
    output.flush()?;
    Ok(header)
}
/// Parses a positive, bounded relative duration for CLI --since.
pub fn since(value: &str) -> Option<i64> {
    let (number, unit) = value.split_at_checked(value.len().checked_sub(1)?)?;
    let seconds = number.parse::<u64>().ok()?.checked_mul(match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86400,
        _ => return None,
    })?;
    if seconds == 0 {
        return None;
    }
    now_ms().checked_sub(i64::try_from(seconds.checked_mul(1000)?).ok()?)
}

// Uses a fixed-size line buffer even for corrupt multi-megabyte lines. Partial
// first/last lines are never exported as records.
fn scan(
    reader: impl Read,
    skip_first: bool,
    mut visit: impl FnMut(&[u8]),
    omitted: &mut u64,
) -> io::Result<()> {
    let mut reader = BufReader::new(reader);
    let mut line = Vec::with_capacity(MAX_RECORD_BYTES);
    let mut discard = skip_first;
    loop {
        let bytes = reader.fill_buf()?;
        if bytes.is_empty() {
            if !line.is_empty() || discard {
                *omitted += 1;
            }
            return Ok(());
        }
        let end = bytes
            .iter()
            .position(|&b| b == b'\n')
            .map_or(bytes.len(), |i| i + 1);
        let complete = bytes[end - 1] == b'\n';
        if line.len() + end > MAX_RECORD_BYTES {
            discard = true;
        }
        if !discard {
            line.extend_from_slice(&bytes[..end]);
        }
        reader.consume(end);
        if complete {
            if discard {
                *omitted += 1;
            } else {
                visit(&line);
            }
            line.clear();
            discard = false;
        }
    }
}
