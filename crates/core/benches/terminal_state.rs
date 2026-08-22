//! Stable Foundation resource measurement executable.

use std::env;
use std::hint::black_box;
use std::process::ExitCode;
use std::time::Instant;

use zterm_core::terminal::{
    TerminalDeltaResult, TerminalModel, TerminalResourceProjection, TerminalSize, TerminalSnapshot,
};

const DEFAULT_SCROLLBACK_ROWS: usize = 10_000;
const DEFAULT_WORKLOAD_LINES: usize = 512;
const INGEST_CHUNK_BYTES: usize = 8 * 1024;

fn main() -> ExitCode {
    let arguments: Vec<String> = env::args().skip(1).collect();
    if arguments.is_empty()
        || arguments
            .iter()
            .any(|argument| matches!(argument.as_str(), "--test" | "--nocapture" | "--list"))
    {
        println!("TERMINAL_BENCH=SKIPPED_TEST_MODE");
        return ExitCode::SUCCESS;
    }

    let result = if arguments == ["--bench"] {
        run_default_matrix()
    } else {
        Options::parse(arguments.into_iter()).and_then(run_one)
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("terminal resource benchmark failed: {error}");
            ExitCode::FAILURE
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Options {
    sessions: usize,
    size: TerminalSize,
    scrollback_rows: usize,
    workload_lines: usize,
}

impl Options {
    fn parse(mut arguments: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut sessions = None;
        let mut rows = None;
        let mut columns = None;
        let mut scrollback_rows = None;
        let mut workload_lines = None;
        while let Some(flag) = arguments.next() {
            let value = arguments
                .next()
                .ok_or_else(|| format!("missing value for {flag}"))?;
            let value = value
                .parse::<usize>()
                .map_err(|error| format!("invalid value for {flag}: {error}"))?;
            match flag.as_str() {
                "--sessions" => sessions = Some(value),
                "--rows" => rows = Some(value),
                "--columns" => columns = Some(value),
                "--scrollback" => scrollback_rows = Some(value),
                "--workload-lines" => workload_lines = Some(value),
                _ => return Err(format!("unknown argument: {flag}")),
            }
        }

        let rows = checked_u16(rows, "--rows")?;
        let columns = checked_u16(columns, "--columns")?;
        let options = Self {
            sessions: positive(sessions, "--sessions")?,
            size: TerminalSize::new(rows, columns),
            scrollback_rows: scrollback_rows.ok_or_else(|| "missing --scrollback".to_owned())?,
            workload_lines: positive(workload_lines, "--workload-lines")?,
        };
        if options.sessions > 64 {
            return Err("--sessions exceeds the measurement safety cap of 64".into());
        }
        Ok(options)
    }
}

fn run_default_matrix() -> Result<(), String> {
    for sessions in [1, 3, 16] {
        for size in [TerminalSize::new(40, 120), TerminalSize::new(256, 512)] {
            run_one(Options {
                sessions,
                size,
                scrollback_rows: DEFAULT_SCROLLBACK_ROWS,
                workload_lines: DEFAULT_WORKLOAD_LINES,
            })?;
        }
    }
    Ok(())
}

fn run_one(options: Options) -> Result<(), String> {
    let workload = mixed_workload(options.workload_lines);
    let started = Instant::now();
    let mut models = Vec::with_capacity(options.sessions);
    let mut checkpoints = Vec::with_capacity(options.sessions);

    for session in 0..options.sessions {
        let mut model = TerminalModel::new(options.size, options.scrollback_rows)
            .map_err(|error| error.to_string())?;
        model
            .ingest(format!("zterm-benchmark-session-{session}\r\n").as_bytes())
            .map_err(|error| error.to_string())?;
        checkpoints.push(model.checkpoint());
        for chunk in workload.chunks(INGEST_CHUNK_BYTES) {
            model.ingest(chunk).map_err(|error| error.to_string())?;
        }
        models.push(model);
    }

    let mut snapshot_bytes = 0_usize;
    let mut delta_bytes = 0_usize;
    let mut resyncs = 0_usize;
    let mut structural_bytes = 0_usize;
    let mut final_revision = 0_u64;
    let mut minimum_retained_history_rows = None;
    for (model, checkpoint) in models.iter().zip(&checkpoints) {
        let snapshot = model.snapshot();
        let retained_history_rows = retained_history_rows(&snapshot)?;
        minimum_retained_history_rows = Some(
            minimum_retained_history_rows.map_or(retained_history_rows, |minimum: usize| {
                minimum.min(retained_history_rows)
            }),
        );
        snapshot_bytes = snapshot_bytes
            .checked_add(snapshot.ansi_payload_len())
            .ok_or_else(|| "snapshot byte count overflow".to_owned())?;
        match model.delta_or_resync(checkpoint) {
            TerminalDeltaResult::Delta(delta) => {
                delta_bytes = delta_bytes
                    .checked_add(delta.ansi_payload_len())
                    .ok_or_else(|| "delta byte count overflow".to_owned())?;
            }
            TerminalDeltaResult::Resync(snapshot) => {
                resyncs += 1;
                delta_bytes = delta_bytes
                    .checked_add(snapshot.ansi_payload_len())
                    .ok_or_else(|| "resync byte count overflow".to_owned())?;
            }
        }
        structural_bytes = structural_bytes
            .checked_add(model.resource_projection().estimated_cell_storage_bytes)
            .ok_or_else(|| "structural byte count overflow".to_owned())?;
        final_revision = final_revision.max(model.revision().get());
    }
    black_box(&models);
    let elapsed_ns = started.elapsed().as_nanos();
    let projection = models
        .first()
        .map(TerminalModel::resource_projection)
        .ok_or_else(|| "benchmark created no models".to_owned())?;

    print_result(
        options,
        projection,
        workload.len(),
        elapsed_ns,
        snapshot_bytes,
        delta_bytes,
        resyncs,
        structural_bytes,
        final_revision,
        minimum_retained_history_rows
            .ok_or_else(|| "benchmark measured no terminal histories".to_owned())?,
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn print_result(
    options: Options,
    projection: TerminalResourceProjection,
    input_bytes: usize,
    elapsed_ns: u128,
    snapshot_bytes: usize,
    delta_bytes: usize,
    resyncs: usize,
    structural_bytes: usize,
    final_revision: u64,
    retained_history_rows_per_session: usize,
) {
    println!(
        "TERMINAL_BENCH sessions={} rows={} columns={} scrollback={} workload=mixed workload_lines={} input_bytes={} elapsed_ns={} snapshot_bytes={} delta_or_resync_bytes={} resyncs={} structural_bytes={} structural_bytes_per_session={} total_cell_capacity_per_session={} retained_history_rows_per_session={} final_revision={}",
        options.sessions,
        options.size.rows,
        options.size.columns,
        options.scrollback_rows,
        options.workload_lines,
        input_bytes,
        elapsed_ns,
        snapshot_bytes,
        delta_bytes,
        resyncs,
        structural_bytes,
        projection.estimated_cell_storage_bytes,
        projection.total_cell_capacity,
        retained_history_rows_per_session,
        final_revision,
    );
}

fn retained_history_rows(snapshot: &TerminalSnapshot) -> Result<usize, String> {
    if snapshot.recent_history_ansi.is_empty() {
        return Ok(0);
    }
    let line_terminators = snapshot
        .recent_history_ansi
        .windows(2)
        .filter(|window| *window == b"\r\n")
        .count();
    line_terminators
        .checked_sub(usize::from(snapshot.size.rows).saturating_sub(1))
        .ok_or_else(|| "snapshot history omitted its replay padding".to_owned())
}

fn mixed_workload(lines: usize) -> Vec<u8> {
    let mut output = Vec::with_capacity(lines.saturating_mul(96));
    for line in 0..lines {
        match line % 4 {
            0 => output.extend_from_slice(format!("ascii-{line:06}-foundation\r\n").as_bytes()),
            1 => output.extend_from_slice(
                format!("\x1b[38;5;{}mcolor-{line:06}\x1b[0m\r\n", line % 256).as_bytes(),
            ),
            2 => output.extend_from_slice(format!("unicode-{line:06}-界-e\u{301}\r\n").as_bytes()),
            _ => output.extend_from_slice(
                format!("\r\x1b[2Khigh-update-{line:06}\x1b[48;2;1;2;3m \x1b[0m\r\n").as_bytes(),
            ),
        }
    }
    output
}

fn positive(value: Option<usize>, flag: &str) -> Result<usize, String> {
    match value {
        Some(0) => Err(format!("{flag} must be positive")),
        Some(value) => Ok(value),
        None => Err(format!("missing {flag}")),
    }
}

fn checked_u16(value: Option<usize>, flag: &str) -> Result<u16, String> {
    let value = positive(value, flag)?;
    value.try_into().map_err(|_| format!("{flag} exceeds u16"))
}
