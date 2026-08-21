//! Generic external full-screen terminal compatibility adapter.

#[cfg(unix)]
use std::ffi::OsString;
#[cfg(unix)]
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::thread;
#[cfg(unix)]
use std::time::{Duration, Instant};

#[cfg(unix)]
use zterm_core::terminal::{ActiveScreen, TerminalDeltaResult, TerminalModel, TerminalSize};
#[cfg(unix)]
use zterm_daemon::terminal_driver::{TerminalDriver, TerminalDriverConfig};
#[cfg(unix)]
use zterm_platform::pty::{ExplicitPtyCommand, PtyChildState, PtyHost, PtySize};

#[cfg(unix)]
const DEADLINE: Duration = Duration::from_secs(20);
#[cfg(unix)]
const SCROLLBACK_ROWS: usize = 10_000;
#[cfg(unix)]
const INITIAL_SIZE: TerminalSize = TerminalSize::new(24, 80);
#[cfg(unix)]
const RESIZED: TerminalSize = TerminalSize::new(47, 123);

fn main() {
    #[cfg(unix)]
    {
        let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
        if arguments.is_empty() {
            println!("TERMINAL_BLACKBOX_GATE=SKIPPED_EXPLICIT_ONLY");
            return;
        }
        let options = match Options::parse(arguments.into_iter()) {
            Ok(options) => options,
            Err(error) => {
                eprintln!("terminal black-box arguments failed: {error}");
                std::process::exit(2);
            }
        };
        if let Err(error) = run(options) {
            eprintln!("terminal black-box gate failed: {error}");
            std::process::exit(1);
        }
    }

    #[cfg(not(unix))]
    println!("TERMINAL_BLACKBOX_GATE=SKIPPED_NON_UNIX");
}

#[cfg(unix)]
struct Options {
    case_name: String,
    program: PathBuf,
    arguments: Vec<OsString>,
    cwd: PathBuf,
    mode: ExerciseMode,
    expected_screen: ActiveScreen,
    quit_sequence: Vec<Vec<u8>>,
}

#[cfg(unix)]
enum ExerciseMode {
    Interaction {
        interaction_file: PathBuf,
        resize_marker: PathBuf,
        completion_marker: PathBuf,
        expected_latest: String,
    },
    Startup,
}

#[cfg(unix)]
impl Options {
    fn parse(arguments: impl Iterator<Item = OsString>) -> Result<Self, String> {
        let mut arguments = arguments;
        let mut case_name = None;
        let mut program = None;
        let mut program_arguments = Vec::new();
        let mut cwd = None;
        let mut mode = None;
        let mut expected_screen = None;
        let mut interaction_file = None;
        let mut resize_marker = None;
        let mut completion_marker = None;
        let mut expected_latest = None;
        let mut quit_sequence = Vec::new();

        while let Some(flag) = arguments.next() {
            let value = arguments
                .next()
                .ok_or_else(|| format!("missing value for {flag:?}"))?;
            match flag.to_str() {
                Some("--case") => case_name = value.into_string().ok(),
                Some("--program") => program = Some(PathBuf::from(value)),
                Some("--arg") => program_arguments.push(value),
                Some("--cwd") => cwd = Some(PathBuf::from(value)),
                Some("--mode") => mode = value.into_string().ok(),
                Some("--expect-screen") => expected_screen = value.into_string().ok(),
                Some("--interaction-file") => interaction_file = Some(PathBuf::from(value)),
                Some("--resize-marker") => resize_marker = Some(PathBuf::from(value)),
                Some("--completion-marker") => completion_marker = Some(PathBuf::from(value)),
                Some("--expect-latest") => expected_latest = value.into_string().ok(),
                Some("--quit-hex") => {
                    quit_sequence.push(decode_hex(
                        value
                            .to_str()
                            .ok_or_else(|| "quit hex must be UTF-8".to_owned())?,
                    )?);
                }
                _ => return Err(format!("unknown argument flag: {flag:?}")),
            }
        }

        let mode = match mode.as_deref() {
            Some("interaction") => ExerciseMode::Interaction {
                interaction_file: absolute(interaction_file, "--interaction-file")?,
                resize_marker: absolute(resize_marker, "--resize-marker")?,
                completion_marker: absolute(completion_marker, "--completion-marker")?,
                expected_latest: expected_latest
                    .ok_or_else(|| "missing --expect-latest".to_owned())?,
            },
            Some("startup") => {
                if interaction_file.is_some()
                    || resize_marker.is_some()
                    || completion_marker.is_some()
                    || expected_latest.is_some()
                {
                    return Err("startup mode does not accept interaction markers".into());
                }
                ExerciseMode::Startup
            }
            Some(mode) => return Err(format!("unsupported --mode: {mode:?}")),
            None => return Err("missing --mode".into()),
        };
        let expected_screen = match expected_screen.as_deref() {
            Some("main") => ActiveScreen::Main,
            Some("alternate") => ActiveScreen::Alternate,
            Some(screen) => return Err(format!("unsupported --expect-screen: {screen:?}")),
            None => return Err("missing --expect-screen".into()),
        };
        if quit_sequence.is_empty() || quit_sequence.len() > 8 {
            return Err("provide between one and eight --quit-hex values".into());
        }

        Ok(Self {
            case_name: case_name.ok_or_else(|| "missing --case".to_owned())?,
            program: absolute(program, "--program")?,
            arguments: program_arguments,
            cwd: absolute(cwd, "--cwd")?,
            mode,
            expected_screen,
            quit_sequence,
        })
    }
}

#[cfg(unix)]
fn run(options: Options) -> Result<(), String> {
    let mut command = ExplicitPtyCommand::new(&options.program, &options.cwd);
    for argument in &options.arguments {
        command = command.arg(argument);
    }
    let session = PtyHost::new()
        .spawn(
            command,
            PtySize::new(INITIAL_SIZE.rows, INITIAL_SIZE.columns),
        )
        .map_err(display_error)?;
    let model = TerminalModel::new(INITIAL_SIZE, SCROLLBACK_ROWS).map_err(display_error)?;
    let driver = TerminalDriver::start(
        session,
        model,
        TerminalDriverConfig {
            byte_channel_capacity: 2,
            read_chunk_bytes: 512,
        },
    )
    .map_err(display_error)?;

    let result = exercise(&driver, &options);
    if result.is_err() {
        let _ = driver.close_explicitly();
    }
    result
}

#[cfg(unix)]
fn exercise(driver: &TerminalDriver, options: &Options) -> Result<(), String> {
    let mut initial = driver.attach();
    let initial_snapshot = wait_for_screen(driver, &mut initial, options.expected_screen)?;
    let initial_revision = initial_snapshot.revision;
    drop(initial);
    if driver.stats().map_err(display_error)?.active_attachments != 0 {
        return Err("outer attachment drop retained a subscriber".into());
    }

    let detached_chunk_baseline = driver.stats().map_err(display_error)?.processed_chunks;
    let resized_revision = driver.resize(RESIZED).map_err(display_error)?;
    if let ExerciseMode::Interaction {
        interaction_file,
        resize_marker,
        completion_marker,
        ..
    } = &options.mode
    {
        let interaction = std::fs::read(interaction_file).map_err(display_error)?;
        driver.write_input(&interaction).map_err(display_error)?;
        wait_for_file(resize_marker, driver)?;
        let resize = std::fs::read_to_string(resize_marker).map_err(display_error)?;
        let (child_rows, child_columns) = parse_size(&resize)?;
        if child_rows <= INITIAL_SIZE.rows
            || child_rows > RESIZED.rows
            || child_columns <= INITIAL_SIZE.columns
            || child_columns > RESIZED.columns
        {
            return Err(format!("child observed unexpected resize: {resize:?}"));
        }
        wait_for_file(completion_marker, driver)?;
    } else {
        wait_for_detached_progress(driver, detached_chunk_baseline)?;
    }
    if driver.stats().map_err(display_error)?.active_attachments != 0 {
        return Err("black-box output required an attachment to complete".into());
    }

    let mut reattached = driver.attach();
    let mut snapshot = match reattached.sync_latest().map_err(display_error)? {
        TerminalDeltaResult::Resync(snapshot) => snapshot,
        TerminalDeltaResult::Delta(_) => {
            return Err("new attachment did not receive a snapshot".into());
        }
    };
    if let ExerciseMode::Interaction {
        expected_latest, ..
    } = &options.mode
    {
        snapshot = wait_for_snapshot_marker(driver, &mut reattached, snapshot, expected_latest)?;
    } else {
        snapshot = wait_for_startup_ready(driver, &mut reattached, snapshot)?;
    }
    if snapshot.revision <= initial_revision || snapshot.revision < resized_revision {
        return Err("black-box terminal did not advance while detached".into());
    }
    if snapshot.size != RESIZED {
        return Err(format!("snapshot retained wrong size: {:?}", snapshot.size));
    }
    if snapshot.active_screen != options.expected_screen {
        return Err(format!(
            "latest snapshot used {:?}, expected {:?}",
            snapshot.active_screen, options.expected_screen
        ));
    }
    drop(reattached);

    let stats = driver.stats().map_err(display_error)?;
    let detached_chunks = stats
        .processed_chunks
        .saturating_sub(detached_chunk_baseline);
    if detached_chunks == 0 {
        return Err("black-box program produced no PTY output while detached".into());
    }
    if stats.maximum_pending_chunks > stats.byte_channel_capacity {
        return Err(format!("byte queue exceeded capacity: {stats:?}"));
    }
    if matches!(&options.mode, ExerciseMode::Interaction { .. })
        && detached_chunks <= u64::try_from(stats.byte_channel_capacity).unwrap_or(u64::MAX)
    {
        return Err(format!(
            "detached black-box output did not exceed one queue window: {stats:?}"
        ));
    }

    send_quit_and_wait(driver, &options.quit_sequence)?;
    driver.wait_until_idle(DEADLINE).map_err(display_error)?;
    let screen = screen_label(options.expected_screen);
    match &options.mode {
        ExerciseMode::Interaction { .. } => println!(
            "BLACKBOX_CASE={},screen={},resize=47x123,detached_progress=true,resync=true,processed_chunks={},detached_chunks={},capacity={},max_pending={}",
            options.case_name,
            screen,
            stats.processed_chunks,
            detached_chunks,
            stats.byte_channel_capacity,
            stats.maximum_pending_chunks
        ),
        ExerciseMode::Startup => println!(
            "TUI_SMOKE_CASE={},screen={},resize=47x123,detached_progress=true,resync=true,prompt_sent=false,processed_chunks={},detached_chunks={},capacity={},max_pending={}",
            options.case_name,
            screen,
            stats.processed_chunks,
            detached_chunks,
            stats.byte_channel_capacity,
            stats.maximum_pending_chunks
        ),
    }
    Ok(())
}

#[cfg(unix)]
fn wait_for_snapshot_marker(
    driver: &TerminalDriver,
    attachment: &mut zterm_daemon::terminal_driver::TerminalAttachment,
    mut snapshot: zterm_core::terminal::TerminalSnapshot,
    marker: &str,
) -> Result<zterm_core::terminal::TerminalSnapshot, String> {
    let deadline = Instant::now() + DEADLINE;
    loop {
        if snapshot_text(&snapshot)?.contains(marker) {
            return Ok(snapshot);
        }
        if let PtyChildState::Exited(status) = driver.try_wait().map_err(display_error)? {
            return Err(format!(
                "full-screen program exited before latest marker {marker:?}: {status:?}"
            ));
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "latest snapshot marker deadline elapsed: {marker:?}"
            ));
        }
        thread::sleep(Duration::from_millis(10));
        snapshot = attachment.latest_snapshot().map_err(display_error)?;
    }
}

#[cfg(unix)]
fn wait_for_startup_ready(
    driver: &TerminalDriver,
    attachment: &mut zterm_daemon::terminal_driver::TerminalAttachment,
    mut snapshot: zterm_core::terminal::TerminalSnapshot,
) -> Result<zterm_core::terminal::TerminalSnapshot, String> {
    let ready_at = Instant::now() + Duration::from_secs(3);
    loop {
        if let PtyChildState::Exited(status) = driver.try_wait().map_err(display_error)? {
            return Err(format!(
                "full-screen program exited before startup settled: {status:?}"
            ));
        }
        if Instant::now() >= ready_at {
            if snapshot_text(&snapshot)?.chars().all(char::is_whitespace) {
                return Err("full-screen startup settled without visible content".into());
            }
            return Ok(snapshot);
        }
        thread::sleep(Duration::from_millis(10));
        snapshot = attachment.latest_snapshot().map_err(display_error)?;
    }
}

#[cfg(unix)]
fn wait_for_detached_progress(
    driver: &TerminalDriver,
    processed_chunk_baseline: u64,
) -> Result<(), String> {
    let deadline = Instant::now() + DEADLINE;
    loop {
        if driver.stats().map_err(display_error)?.processed_chunks > processed_chunk_baseline {
            return Ok(());
        }
        if let PtyChildState::Exited(status) = driver.try_wait().map_err(display_error)? {
            return Err(format!(
                "full-screen program exited before detached output: {status:?}"
            ));
        }
        if Instant::now() >= deadline {
            return Err("full-screen program produced no output while detached".into());
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(unix)]
fn wait_for_screen(
    driver: &TerminalDriver,
    attachment: &mut zterm_daemon::terminal_driver::TerminalAttachment,
    expected_screen: ActiveScreen,
) -> Result<zterm_core::terminal::TerminalSnapshot, String> {
    let deadline = Instant::now() + DEADLINE;
    loop {
        let snapshot = attachment.latest_snapshot().map_err(display_error)?;
        if snapshot.active_screen == expected_screen && snapshot.revision > 0 {
            return Ok(snapshot);
        }
        if let PtyChildState::Exited(status) = driver.try_wait().map_err(display_error)? {
            return Err(format!(
                "full-screen program exited during startup: {status:?}"
            ));
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "program did not enter the expected {expected_screen:?} screen"
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(unix)]
fn wait_for_file(path: &Path, driver: &TerminalDriver) -> Result<(), String> {
    let deadline = Instant::now() + DEADLINE;
    loop {
        if path.is_file() {
            return Ok(());
        }
        if let PtyChildState::Exited(status) = driver.try_wait().map_err(display_error)? {
            return Err(format!(
                "full-screen program exited before marker {}: {status:?}",
                path.display()
            ));
        }
        if Instant::now() >= deadline {
            return Err(format!("marker deadline elapsed: {}", path.display()));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(unix)]
fn wait_for_natural_exit(driver: &TerminalDriver) -> Result<(), String> {
    let deadline = Instant::now() + DEADLINE;
    loop {
        match driver.try_wait().map_err(display_error)? {
            PtyChildState::Running if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10));
            }
            PtyChildState::Running => return Err("program did not exit after quit input".into()),
            PtyChildState::Exited(status) if status.success() => return Ok(()),
            PtyChildState::Exited(status) => {
                return Err(format!("program exit was unsuccessful: {status:?}"));
            }
        }
    }
}

#[cfg(unix)]
fn send_quit_and_wait(driver: &TerminalDriver, quit_sequence: &[Vec<u8>]) -> Result<(), String> {
    for (write_index, quit_bytes) in quit_sequence.iter().enumerate() {
        driver.write_input(quit_bytes).map_err(display_error)?;
        if write_index + 1 < quit_sequence.len()
            && wait_for_natural_exit_until(driver, Duration::from_millis(250))?
        {
            return Ok(());
        }
    }
    wait_for_natural_exit(driver)
}

#[cfg(unix)]
fn wait_for_natural_exit_until(
    driver: &TerminalDriver,
    duration: Duration,
) -> Result<bool, String> {
    let deadline = Instant::now() + duration;
    loop {
        match driver.try_wait().map_err(display_error)? {
            PtyChildState::Running if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10));
            }
            PtyChildState::Running => return Ok(false),
            PtyChildState::Exited(status) if status.success() => return Ok(true),
            PtyChildState::Exited(status) => {
                return Err(format!("program exit was unsuccessful: {status:?}"));
            }
        }
    }
}

#[cfg(unix)]
fn snapshot_text(snapshot: &zterm_core::terminal::TerminalSnapshot) -> Result<String, String> {
    let mut client = TerminalModel::new(snapshot.size, SCROLLBACK_ROWS).map_err(display_error)?;
    client
        .ingest(&snapshot.recent_history_ansi)
        .map_err(display_error)?;
    client
        .ingest(&snapshot.screen_ansi)
        .map_err(display_error)?;
    let state = client.state();
    let columns = usize::from(state.size.columns);
    let mut text = String::new();
    for row in state.cells.chunks(columns) {
        for cell in row {
            text.push_str(&cell.contents);
        }
        text.push('\n');
    }
    Ok(text)
}

#[cfg(unix)]
fn absolute(path: Option<PathBuf>, flag: &str) -> Result<PathBuf, String> {
    let path = path.ok_or_else(|| format!("missing {flag}"))?;
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(format!("{flag} must be absolute: {}", path.display()))
    }
}

#[cfg(unix)]
fn decode_hex(value: &str) -> Result<Vec<u8>, String> {
    if !value.len().is_multiple_of(2) {
        return Err("quit hex must contain complete bytes".into());
    }
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let pair = std::str::from_utf8(pair).map_err(display_error)?;
            u8::from_str_radix(pair, 16).map_err(display_error)
        })
        .collect()
}

#[cfg(unix)]
fn parse_size(value: &str) -> Result<(u16, u16), String> {
    let mut fields = value.split_whitespace();
    let rows = fields
        .next()
        .ok_or_else(|| "resize marker omitted rows".to_owned())?
        .parse::<u16>()
        .map_err(display_error)?;
    let columns = fields
        .next()
        .ok_or_else(|| "resize marker omitted columns".to_owned())?
        .parse::<u16>()
        .map_err(display_error)?;
    if fields.next().is_some() {
        return Err(format!("resize marker contained extra fields: {value:?}"));
    }
    Ok((rows, columns))
}

#[cfg(unix)]
const fn screen_label(screen: ActiveScreen) -> &'static str {
    match screen {
        ActiveScreen::Main => "main",
        ActiveScreen::Alternate => "alternate",
    }
}

#[cfg(unix)]
fn display_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}
