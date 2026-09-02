//! Self-child PTY lifecycle integration gate.

#[cfg(unix)]
use std::ffi::OsString;
#[cfg(unix)]
use std::fs::{self, OpenOptions};
#[cfg(unix)]
use std::io::{self, BufRead, Read, Write};
#[cfg(unix)]
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::{Command, Stdio};
#[cfg(unix)]
use std::sync::{Arc, Mutex, mpsc};
#[cfg(unix)]
use std::thread;
#[cfg(unix)]
use std::time::{Duration, Instant, SystemTime};

#[cfg(unix)]
use zterm_platform::pty::{
    ExplicitPtyCommand, PtyChildState, PtyError, PtyHost, PtyReader, PtySession, PtySize,
};

#[cfg(unix)]
const DEADLINE: Duration = Duration::from_secs(10);
#[cfg(unix)]
const BULK_BYTES: usize = 1024 * 1024;

fn main() {
    #[cfg(unix)]
    {
        let args = std::env::args_os().collect::<Vec<_>>();
        if args
            .get(1)
            .is_some_and(|arg| arg == "--hosted-profile-probe")
        {
            let code = match hosted_profile_probe() {
                Ok(()) => 0,
                Err(error) => {
                    eprintln!("hosted profile probe failed: {error}");
                    2
                }
            };
            std::process::exit(code);
        }
        if args.get(1).is_some_and(|arg| arg == "--fixture-child") {
            let code = match fixture_child(&args[2..]) {
                Ok(code) => code,
                Err(error) => {
                    eprintln!("fixture child failed: {error}");
                    2
                }
            };
            std::process::exit(code);
        }

        if let Err(error) = run_unix_gate() {
            eprintln!("PTY lifecycle gate failed: {error}");
            std::process::exit(1);
        }
        println!("PTY_LIFECYCLE_GATE=PASS");
    }

    #[cfg(not(unix))]
    println!("PTY_LIFECYCLE_GATE=SKIPPED_NON_UNIX");
}

#[cfg(unix)]
fn run_unix_gate() -> Result<(), String> {
    hosted_profile_is_independent_of_parent_terminal()?;
    interactive_lifecycle()?;
    high_output_lifecycle()?;
    explicit_close_lifecycle()?;
    Ok(())
}

#[cfg(unix)]
fn hosted_profile_is_independent_of_parent_terminal() -> Result<(), String> {
    for (label, term, colorterm) in [
        ("ghostty", Some("xterm-ghostty"), Some("truecolor")),
        ("kitty", Some("xterm-kitty"), Some("24bit")),
        ("tmux", Some("screen-256color"), None),
        ("unset", None, None),
    ] {
        let executable = current_executable()?;
        let mut command = Command::new(executable);
        command
            .arg("--hosted-profile-probe")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        match term {
            Some(term) => {
                command.env("TERM", term);
            }
            None => {
                command.env_remove("TERM");
            }
        }
        match colorterm {
            Some(colorterm) => {
                command.env("COLORTERM", colorterm);
            }
            None => {
                command.env_remove("COLORTERM");
            }
        }
        let mut child = command.spawn().map_err(display_error)?;
        let deadline = Instant::now() + DEADLINE;
        loop {
            if let Some(status) = child.try_wait().map_err(display_error)? {
                let output = child.wait_with_output().map_err(display_error)?;
                if !status.success() {
                    return Err(format!(
                        "{label} parent profile probe failed: {}",
                        String::from_utf8_lossy(&output.stderr)
                    ));
                }
                require_contains(&output.stdout, b"HOSTED_PROFILE:xterm-256color:truecolor")?;
                break;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "{label} parent profile probe exceeded its deadline"
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
    println!("PTY_CASE=hosted_profile,parent_variants=4,term=xterm-256color");
    Ok(())
}

#[cfg(unix)]
fn hosted_profile_probe() -> Result<(), String> {
    let mut session = PtyHost::new()
        .spawn_current_account_login_shell(PtySize::default(), None)
        .map_err(display_error)?;
    let reader = session.take_reader().map_err(display_error)?;
    let output = OutputDrain::start(reader);
    session
        .write_input(b"printf 'HOSTED_PROFILE:%s:%s\\n' \"$TERM\" \"$COLORTERM\"; exit\n")
        .map_err(display_error)?;
    output.wait_for(b"HOSTED_PROFILE:xterm-256color:truecolor", DEADLINE)?;
    let status = wait_for_exit(&mut session, DEADLINE)?;
    if !status.success() {
        return Err(format!(
            "hosted login shell did not exit successfully: {status:?}"
        ));
    }
    let bytes = output.finish(DEADLINE)?;
    require_contains(&bytes, b"HOSTED_PROFILE:xterm-256color:truecolor")?;
    println!("HOSTED_PROFILE:xterm-256color:truecolor");
    Ok(())
}

#[cfg(unix)]
fn interactive_lifecycle() -> Result<(), String> {
    let executable = current_executable()?;
    let stty = find_stty()?;
    let command = fixture_command(&executable, ["interactive".into(), stty.into_os_string()]);
    let mut fixture = TestSession::spawn(command, PtySize::new(24, 80))?;

    if fixture.session.process_id().is_none() {
        return Err("fixture child did not expose a process identifier".into());
    }
    if !matches!(
        fixture.session.take_reader(),
        Err(PtyError::ReaderAlreadyTaken)
    ) {
        return Err("take_reader did not enforce one transfer".into());
    }
    if fixture.session.try_wait().map_err(display_error)? != PtyChildState::Running {
        return Err("interactive child exited before receiving input".into());
    }

    fixture.wait_for_output(b"FIXTURE-READY")?;
    fixture
        .session
        .write_input(b"echo:ordered-input\n")
        .map_err(display_error)?;
    fixture.wait_for_output(b"INPUT:ordered-input")?;

    fixture
        .session
        .resize(PtySize::new(47, 123))
        .map_err(display_error)?;
    fixture
        .session
        .write_input(b"report-size\n")
        .map_err(display_error)?;
    fixture.wait_for_output(b"SIZE:47x123")?;

    fixture
        .session
        .write_input(b"exit\n")
        .map_err(display_error)?;
    fixture.wait_for_output(b"NATURAL-EXIT")?;
    let status = wait_for_exit(&mut fixture.session, DEADLINE)?;
    if status.exit_code() != 23 || status.signal().is_some() || status.success() {
        return Err(format!("unexpected natural exit status: {status:?}"));
    }
    let terminal_wait = fixture.session.wait().map_err(display_error)?;
    if terminal_wait != status {
        return Err("terminal wait did not return the cached natural status".into());
    }

    let output = fixture.finish_output()?;
    require_contains(&output, b"INPUT:ordered-input")?;
    require_contains(&output, b"SIZE:47x123")?;
    println!("PTY_CASE=interactive,natural_exit=23,resize=47x123");
    Ok(())
}

#[cfg(unix)]
fn high_output_lifecycle() -> Result<(), String> {
    let executable = current_executable()?;
    let marker = CompletionMarker::new()?;
    let command = fixture_command(
        &executable,
        [
            "bulk".into(),
            marker.path().as_os_str().to_owned(),
            BULK_BYTES.to_string().into(),
        ],
    );
    let mut fixture = TestSession::spawn(command, PtySize::default())?;

    wait_for_marker(marker.path(), &mut fixture.session, DEADLINE)?;
    let marker_contents = fs::read(marker.path()).map_err(display_error)?;
    if marker_contents != b"complete" {
        return Err("fixture completion marker had unexpected contents".into());
    }

    let status = wait_for_exit(&mut fixture.session, DEADLINE)?;
    if !status.success() {
        return Err(format!(
            "bulk fixture did not exit successfully: {status:?}"
        ));
    }
    let output = fixture.finish_output()?;
    if output.iter().filter(|byte| **byte == b'X').count() < BULK_BYTES {
        return Err(format!(
            "bulk output was truncated: read {} total bytes",
            output.len()
        ));
    }
    require_contains(&output, b"BULK-COMPLETE")?;
    println!("PTY_CASE=bulk,bytes={BULK_BYTES},marker=complete");
    Ok(())
}

#[cfg(unix)]
fn explicit_close_lifecycle() -> Result<(), String> {
    let executable = current_executable()?;
    let command = fixture_command(&executable, [OsString::from("block")]);
    let mut fixture = TestSession::spawn(command, PtySize::default())?;
    fixture.wait_for_output(b"BLOCKING")?;

    let started = Instant::now();
    let status = fixture.session.close_explicitly().map_err(display_error)?;
    if started.elapsed() > DEADLINE {
        return Err("explicit close exceeded its deadline".into());
    }
    if status.success() || status.signal().is_none() {
        return Err(format!("explicit close was not signal-driven: {status:?}"));
    }
    if fixture.session.try_wait().map_err(display_error)? != PtyChildState::Exited(status.clone()) {
        return Err("try_wait did not retain explicit-close status".into());
    }
    let output = fixture.finish_output()?;
    require_contains(&output, b"BLOCKING")?;
    println!("PTY_CASE=explicit_close,signal={:?}", status.signal());
    Ok(())
}

#[cfg(unix)]
fn fixture_child(arguments: &[OsString]) -> Result<i32, String> {
    match arguments.first().and_then(|argument| argument.to_str()) {
        Some("interactive") => {
            let stty = arguments
                .get(1)
                .map(PathBuf::from)
                .ok_or_else(|| "interactive fixture requires stty path".to_owned())?;
            interactive_child(&stty)
        }
        Some("bulk") => {
            let marker = arguments
                .get(1)
                .map(PathBuf::from)
                .ok_or_else(|| "bulk fixture requires marker path".to_owned())?;
            let count = arguments
                .get(2)
                .and_then(|argument| argument.to_str())
                .ok_or_else(|| "bulk fixture requires byte count".to_owned())?
                .parse::<usize>()
                .map_err(display_error)?;
            bulk_child(&marker, count)
        }
        Some("block") => blocking_child(),
        mode => Err(format!("unknown fixture mode: {mode:?}")),
    }
}

#[cfg(unix)]
fn interactive_child(stty: &Path) -> Result<i32, String> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let stdout = io::stdout();
    let mut output = stdout.lock();
    writeln!(output, "FIXTURE-READY").map_err(display_error)?;
    output.flush().map_err(display_error)?;

    let mut line = String::new();
    loop {
        line.clear();
        if input.read_line(&mut line).map_err(display_error)? == 0 {
            return Err("interactive fixture received EOF".into());
        }
        let command = line.trim_end_matches(['\r', '\n']);
        if let Some(value) = command.strip_prefix("echo:") {
            writeln!(output, "INPUT:{value}").map_err(display_error)?;
            output.flush().map_err(display_error)?;
        } else if command == "report-size" {
            let size = observed_terminal_size(stty)?;
            writeln!(output, "SIZE:{}x{}", size.0, size.1).map_err(display_error)?;
            output.flush().map_err(display_error)?;
        } else if command == "exit" {
            writeln!(output, "NATURAL-EXIT").map_err(display_error)?;
            output.flush().map_err(display_error)?;
            return Ok(23);
        } else {
            return Err(format!("unexpected interactive command: {command:?}"));
        }
    }
}

#[cfg(unix)]
fn observed_terminal_size(stty: &Path) -> Result<(u16, u16), String> {
    let output = Command::new(stty)
        .arg("size")
        .stdin(Stdio::inherit())
        .output()
        .map_err(display_error)?;
    if !output.status.success() {
        return Err(format!(
            "stty size failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let text = String::from_utf8(output.stdout).map_err(display_error)?;
    let mut fields = text.split_whitespace();
    let rows = fields
        .next()
        .ok_or_else(|| "stty omitted rows".to_owned())?
        .parse::<u16>()
        .map_err(display_error)?;
    let columns = fields
        .next()
        .ok_or_else(|| "stty omitted columns".to_owned())?
        .parse::<u16>()
        .map_err(display_error)?;
    if fields.next().is_some() {
        return Err(format!("stty returned extra fields: {text:?}"));
    }
    Ok((rows, columns))
}

#[cfg(unix)]
fn bulk_child(marker: &Path, count: usize) -> Result<i32, String> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    let chunk = [b'X'; 8192];
    let mut remaining = count;
    while remaining > 0 {
        let write_count = remaining.min(chunk.len());
        output
            .write_all(&chunk[..write_count])
            .map_err(display_error)?;
        remaining -= write_count;
    }
    output.flush().map_err(display_error)?;

    let mut marker_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(marker)
        .map_err(display_error)?;
    marker_file.write_all(b"complete").map_err(display_error)?;
    marker_file.sync_all().map_err(display_error)?;
    writeln!(output, "BULK-COMPLETE").map_err(display_error)?;
    output.flush().map_err(display_error)?;
    Ok(0)
}

#[cfg(unix)]
fn blocking_child() -> Result<i32, String> {
    println!("BLOCKING");
    io::stdout().flush().map_err(display_error)?;
    loop {
        thread::park_timeout(Duration::from_secs(60));
    }
}

#[cfg(unix)]
fn current_executable() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(display_error)
}

#[cfg(unix)]
fn fixture_command<const N: usize>(
    executable: &Path,
    arguments: [OsString; N],
) -> ExplicitPtyCommand {
    let mut command =
        ExplicitPtyCommand::new(executable, std::env::temp_dir()).arg("--fixture-child");
    for argument in arguments {
        command = command.arg(argument);
    }
    command
}

#[cfg(unix)]
fn find_stty() -> Result<PathBuf, String> {
    [PathBuf::from("/bin/stty"), PathBuf::from("/usr/bin/stty")]
        .into_iter()
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| "neither /bin/stty nor /usr/bin/stty exists".into())
}

#[cfg(unix)]
fn wait_for_exit(
    session: &mut PtySession,
    timeout: Duration,
) -> Result<zterm_platform::pty::PtyExitStatus, String> {
    let deadline = Instant::now() + timeout;
    loop {
        match session.try_wait().map_err(display_error)? {
            PtyChildState::Running if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10));
            }
            PtyChildState::Running => return Err("child exit deadline elapsed".into()),
            PtyChildState::Exited(status) => return Ok(status),
        }
    }
}

#[cfg(unix)]
fn wait_for_marker(
    marker: &Path,
    session: &mut PtySession,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        if marker.is_file() {
            return Ok(());
        }
        if let PtyChildState::Exited(status) = session.try_wait().map_err(display_error)? {
            return Err(format!(
                "bulk child exited before completion marker: {status:?}"
            ));
        }
        if Instant::now() >= deadline {
            return Err("completion marker deadline elapsed".into());
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(unix)]
fn require_contains(output: &[u8], needle: &[u8]) -> Result<(), String> {
    if output.windows(needle.len()).any(|window| window == needle) {
        Ok(())
    } else {
        Err(format!(
            "output omitted {:?}; tail={:?}",
            String::from_utf8_lossy(needle),
            output_tail(output)
        ))
    }
}

#[cfg(unix)]
fn output_tail(output: &[u8]) -> String {
    let start = output.len().saturating_sub(256);
    String::from_utf8_lossy(&output[start..]).into_owned()
}

#[cfg(unix)]
fn display_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(unix)]
struct OutputDrain {
    bytes: Arc<Mutex<Vec<u8>>>,
    completed: mpsc::Receiver<Result<(), String>>,
    thread: Option<thread::JoinHandle<()>>,
}

#[cfg(unix)]
impl OutputDrain {
    fn start(mut reader: PtyReader) -> Self {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let thread_bytes = Arc::clone(&bytes);
        let (sender, completed) = mpsc::channel();
        let thread = thread::spawn(move || {
            let result = (|| {
                let mut buffer = [0_u8; 8192];
                loop {
                    let count = reader.read(&mut buffer).map_err(display_error)?;
                    if count == 0 {
                        return Ok(());
                    }
                    thread_bytes
                        .lock()
                        .map_err(|_| "output buffer lock was poisoned".to_owned())?
                        .extend_from_slice(&buffer[..count]);
                }
            })();
            let _ = sender.send(result);
        });
        Self {
            bytes,
            completed,
            thread: Some(thread),
        }
    }

    fn wait_for(&self, needle: &[u8], timeout: Duration) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        loop {
            let bytes = self
                .bytes
                .lock()
                .map_err(|_| "output buffer lock was poisoned".to_owned())?;
            if bytes.windows(needle.len()).any(|window| window == needle) {
                return Ok(());
            }
            let tail = output_tail(&bytes);
            drop(bytes);
            if Instant::now() >= deadline {
                return Err(format!(
                    "output deadline elapsed waiting for {:?}; tail={tail:?}",
                    String::from_utf8_lossy(needle)
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn finish(mut self, timeout: Duration) -> Result<Vec<u8>, String> {
        let read_result = self
            .completed
            .recv_timeout(timeout)
            .map_err(|error| format!("PTY reader did not finish: {error}"))?;
        if let Some(handle) = self.thread.take() {
            handle
                .join()
                .map_err(|_| "PTY reader thread panicked".to_owned())?;
        }
        read_result?;
        self.bytes
            .lock()
            .map_err(|_| "output buffer lock was poisoned".to_owned())
            .map(|bytes| bytes.clone())
    }
}

#[cfg(unix)]
struct TestSession {
    session: PtySession,
    output: Option<OutputDrain>,
}

#[cfg(unix)]
impl TestSession {
    fn spawn(command: ExplicitPtyCommand, size: PtySize) -> Result<Self, String> {
        let mut session = PtyHost::new().spawn(command, size).map_err(display_error)?;
        let reader = session.take_reader().map_err(display_error)?;
        Ok(Self {
            session,
            output: Some(OutputDrain::start(reader)),
        })
    }

    fn wait_for_output(&self, needle: &[u8]) -> Result<(), String> {
        self.output
            .as_ref()
            .ok_or_else(|| "PTY output drain was already finished".to_owned())?
            .wait_for(needle, DEADLINE)
    }

    fn finish_output(&mut self) -> Result<Vec<u8>, String> {
        let output = self
            .output
            .take()
            .ok_or_else(|| "PTY output drain was already finished".to_owned())?;
        output.finish(DEADLINE)
    }
}

#[cfg(unix)]
impl Drop for TestSession {
    fn drop(&mut self) {
        let _ = self.session.close_explicitly();
        if let Some(output) = self.output.take() {
            let _ = output.finish(DEADLINE);
        }
    }
}

#[cfg(unix)]
struct CompletionMarker(PathBuf);

#[cfg(unix)]
impl CompletionMarker {
    fn new() -> Result<Self, String> {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(display_error)?
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("zterm-pty-marker-{}-{nonce}", std::process::id()));
        if path.exists() {
            return Err(format!(
                "completion marker already exists: {}",
                path.display()
            ));
        }
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

#[cfg(unix)]
impl Drop for CompletionMarker {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_file(&self.0)
            && error.kind() != io::ErrorKind::NotFound
        {
            eprintln!(
                "failed to clean completion marker {}: {error}",
                self.0.display()
            );
        }
    }
}
