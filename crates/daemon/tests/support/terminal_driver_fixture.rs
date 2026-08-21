use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use zterm_core::terminal::{TerminalModel, TerminalSize};
use zterm_daemon::terminal_driver::{TerminalDriver, TerminalDriverConfig};
use zterm_platform::pty::{ExplicitPtyCommand, PtyChildState, PtyHost, PtySize};

pub const DEADLINE: Duration = Duration::from_secs(15);
pub const SCROLLBACK_ROWS: usize = 10_000;
pub const INITIAL_SIZE: TerminalSize = TerminalSize::new(24, 80);

pub fn maybe_run_fixture_child() -> bool {
    let arguments = std::env::args_os().collect::<Vec<_>>();
    if arguments
        .get(1)
        .is_none_or(|value| value != "--fixture-child")
    {
        return false;
    }
    let result = fixture_child(&arguments[2..]);
    match result {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("terminal driver fixture failed: {error}");
            std::process::exit(2);
        }
    }
}

pub fn spawn_driver<const N: usize>(
    arguments: [OsString; N],
    config: TerminalDriverConfig,
) -> Result<TerminalDriver, String> {
    let executable = std::env::current_exe().map_err(display_error)?;
    let mut command =
        ExplicitPtyCommand::new(executable, std::env::temp_dir()).arg("--fixture-child");
    for argument in arguments {
        command = command.arg(argument);
    }
    let session = PtyHost::new()
        .spawn(
            command,
            PtySize::new(INITIAL_SIZE.rows, INITIAL_SIZE.columns),
        )
        .map_err(display_error)?;
    let model = TerminalModel::new(INITIAL_SIZE, SCROLLBACK_ROWS).map_err(display_error)?;
    TerminalDriver::start(session, model, config).map_err(display_error)
}

pub fn wait_for_marker(marker: &Path, driver: &TerminalDriver) -> Result<(), String> {
    let deadline = Instant::now() + DEADLINE;
    loop {
        if marker.is_file() {
            return Ok(());
        }
        if let PtyChildState::Exited(status) = driver.try_wait().map_err(display_error)? {
            return Err(format!(
                "fixture exited before marker {}: {status:?}",
                marker.display()
            ));
        }
        if Instant::now() >= deadline {
            return Err(format!("marker deadline elapsed: {}", marker.display()));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

pub fn wait_for_text(driver: &TerminalDriver, needle: &str) -> Result<(), String> {
    let deadline = Instant::now() + DEADLINE;
    loop {
        let attachment = driver.attach();
        let snapshot = attachment.latest_snapshot().map_err(display_error)?;
        let text = snapshot_text(&snapshot)?;
        if text.contains(needle) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "snapshot deadline elapsed waiting for {needle:?}; tail={:?}",
                text.chars().rev().take(160).collect::<String>()
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

pub fn snapshot_text(snapshot: &zterm_core::terminal::TerminalSnapshot) -> Result<String, String> {
    let mut client = TerminalModel::new(snapshot.size, SCROLLBACK_ROWS).map_err(display_error)?;
    client
        .ingest(&snapshot.recent_history_ansi)
        .map_err(display_error)?;
    client
        .ingest(&snapshot.screen_ansi)
        .map_err(display_error)?;
    Ok(state_text(&client))
}

pub fn state_text(model: &TerminalModel) -> String {
    let state = model.state();
    let columns = usize::from(state.size.columns);
    let mut text = String::new();
    for row in state.cells.chunks(columns) {
        for cell in row {
            text.push_str(&cell.contents);
        }
        text.push('\n');
    }
    text
}

pub fn wait_for_natural_exit(driver: &TerminalDriver) -> Result<(), String> {
    let deadline = Instant::now() + DEADLINE;
    loop {
        match driver.try_wait().map_err(display_error)? {
            PtyChildState::Running if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10));
            }
            PtyChildState::Running => return Err("fixture exit deadline elapsed".into()),
            PtyChildState::Exited(status) if status.success() => return Ok(()),
            PtyChildState::Exited(status) => {
                return Err(format!("fixture exit was unsuccessful: {status:?}"));
            }
        }
    }
}

pub struct TempMarker(PathBuf);

impl TempMarker {
    pub fn new(label: &str) -> Result<Self, String> {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(display_error)?
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "zterm-{label}-{}-{nonce}.marker",
            std::process::id()
        ));
        if path.exists() {
            return Err(format!("marker already exists: {}", path.display()));
        }
        Ok(Self(path))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn argument(&self) -> OsString {
        self.0.as_os_str().to_owned()
    }
}

impl Drop for TempMarker {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_file(&self.0)
            && error.kind() != io::ErrorKind::NotFound
        {
            eprintln!("failed to remove marker {}: {error}", self.0.display());
        }
    }
}

fn fixture_child(arguments: &[OsString]) -> Result<i32, String> {
    match arguments.first().and_then(|value| value.to_str()) {
        Some("bulk") => {
            let marker = required_path(arguments, 1, "bulk marker")?;
            let count = required_usize(arguments, 2, "bulk byte count")?;
            bulk_child(&marker, count)
        }
        Some("transport") => command_child("TRANSPORT-READY"),
        Some("query") => query_child(),
        Some("burst") => {
            let marker = required_path(arguments, 1, "burst marker")?;
            burst_child(&marker)
        }
        mode => Err(format!("unknown fixture mode: {mode:?}")),
    }
}

fn bulk_child(marker: &Path, count: usize) -> Result<i32, String> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    let mut written = 0;
    let mut line = vec![b'X'; 4095];
    line.push(b'\n');
    while written < count {
        output.write_all(&line).map_err(display_error)?;
        written += line.len();
    }
    writeln!(output, "BULK-COMPLETE").map_err(display_error)?;
    output.flush().map_err(display_error)?;
    write_marker(marker)?;
    command_loop(&mut output)
}

fn burst_child(marker: &Path) -> Result<i32, String> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let stdout = io::stdout();
    let mut output = stdout.lock();
    writeln!(output, "BASELINE-STATE").map_err(display_error)?;
    output.flush().map_err(display_error)?;
    let mut line = String::new();
    loop {
        line.clear();
        if input.read_line(&mut line).map_err(display_error)? == 0 {
            return Err("burst fixture received EOF".into());
        }
        match line.trim_end_matches(['\r', '\n']) {
            "burst" => {
                for index in 0..4_000 {
                    writeln!(output, "\x1b[3{}mrevision-{index:04}\x1b[0m", index % 8)
                        .map_err(display_error)?;
                }
                writeln!(output, "LATEST-STATE").map_err(display_error)?;
                output.flush().map_err(display_error)?;
                write_marker(marker)?;
            }
            "exit" => return Ok(0),
            command => return Err(format!("unexpected burst command: {command:?}")),
        }
    }
}

fn command_child(ready: &str) -> Result<i32, String> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    writeln!(output, "{ready}").map_err(display_error)?;
    output.flush().map_err(display_error)?;
    command_loop(&mut output)
}

fn query_child() -> Result<i32, String> {
    let stty = [PathBuf::from("/bin/stty"), PathBuf::from("/usr/bin/stty")]
        .into_iter()
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| "query fixture could not find stty".to_owned())?;
    let status = Command::new(stty)
        .args(["raw", "-echo"])
        .stdin(Stdio::inherit())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .status()
        .map_err(display_error)?;
    if !status.success() {
        return Err(format!("stty raw failed: {status}"));
    }

    io::stdout()
        .lock()
        .write_all(b"\x1b[5n")
        .map_err(display_error)?;
    io::stdout().flush().map_err(display_error)?;
    let mut reply = [0_u8; 4];
    io::stdin()
        .lock()
        .read_exact(&mut reply)
        .map_err(display_error)?;
    if reply != *b"\x1b[0n" {
        return Err(format!("unexpected DSR reply: {reply:?}"));
    }
    Ok(0)
}

fn command_loop(output: &mut impl Write) -> Result<i32, String> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut line = String::new();
    loop {
        line.clear();
        if input.read_line(&mut line).map_err(display_error)? == 0 {
            return Err("fixture received EOF".into());
        }
        match line.trim_end_matches(['\r', '\n']) {
            "probe" => {
                writeln!(output, "CHILD-STILL-RUNNING").map_err(display_error)?;
                output.flush().map_err(display_error)?;
            }
            "exit" => return Ok(0),
            command => return Err(format!("unexpected fixture command: {command:?}")),
        }
    }
}

fn write_marker(path: &Path) -> Result<(), String> {
    let mut marker = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(display_error)?;
    marker.write_all(b"complete").map_err(display_error)?;
    marker.sync_all().map_err(display_error)
}

fn required_path(arguments: &[OsString], index: usize, name: &str) -> Result<PathBuf, String> {
    arguments
        .get(index)
        .map(PathBuf::from)
        .ok_or_else(|| format!("fixture requires {name}"))
}

fn required_usize(arguments: &[OsString], index: usize, name: &str) -> Result<usize, String> {
    arguments
        .get(index)
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("fixture requires {name}"))?
        .parse::<usize>()
        .map_err(display_error)
}

pub fn display_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}
