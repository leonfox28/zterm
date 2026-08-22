//! Thin command parsing and rendering over daemon-owned operations.

use std::fmt;
use std::io::{self, IsTerminal, Write};

use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use serde::Serialize;
use zterm_daemon::bootstrap::BootstrapResult;
use zterm_daemon::config::validate_setup_profile;
use zterm_daemon::error::DaemonError;
use zterm_daemon::operations::{DoctorReport, LocalRuntime, ObservedState};
use zterm_daemon::service::DaemonStatus;

/// zterm's M3 public command tree plus one hidden daemon entry flag.
#[derive(Debug, Parser)]
#[command(
    name = "zterm",
    version,
    about = "Secure remote terminal (core/local daemon milestone)",
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Internal detached daemon entry; never accepted as a state-path override.
    #[arg(long, hide = true)]
    internal_daemon: bool,
    /// Public operation to perform.
    #[command(subcommand)]
    command: Option<Command>,
}

impl Cli {
    /// Whether this parse selected the hidden pre-runtime daemon entry.
    #[must_use]
    pub const fn internal_daemon(&self) -> bool {
        self.internal_daemon
    }
}

/// Public M3 commands.
#[derive(Debug, Subcommand)]
enum Command {
    /// Configure this device and explicitly start its daemon.
    Setup(SetupArgs),
    /// Show setup and daemon state without starting anything.
    Status(JsonArgs),
    /// Run local-only diagnostics without starting anything.
    Doctor(JsonArgs),
    /// Inspect or control the per-user daemon.
    Daemon {
        /// Daemon lifecycle operation.
        #[command(subcommand)]
        command: DaemonCommand,
    },
    /// Print a bounded recent daemon log tail without starting anything.
    Logs(LogsArgs),
}

#[derive(Debug, clap::Args)]
struct SetupArgs {
    /// User-facing device name.
    #[arg(long)]
    name: Option<String>,
    /// Infrastructure profile; official n0 is the recommended default.
    #[arg(long, value_enum)]
    profile: Option<ProfileArg>,
    /// HTTPS Relay URL required only by self-hosted profile.
    #[arg(long)]
    relay_url: Option<String>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ProfileArg {
    OfficialN0,
    SelfHosted,
}

impl ProfileArg {
    const fn as_str(self) -> &'static str {
        match self {
            Self::OfficialN0 => "official-n0",
            Self::SelfHosted => "self-hosted",
        }
    }
}

#[derive(Debug, clap::Args)]
struct JsonArgs {
    /// Render the same typed result as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Subcommand)]
enum DaemonCommand {
    /// Show daemon state without starting it.
    Status(JsonArgs),
    /// Gracefully stop the daemon; already stopped succeeds.
    Stop(ForceArgs),
    /// Gracefully stop and explicitly start one daemon.
    Restart(ForceArgs),
}

#[derive(Debug, clap::Args)]
struct ForceArgs {
    /// Allow interruption when future milestones report active sessions.
    #[arg(long)]
    force: bool,
}

#[derive(Debug, clap::Args)]
struct LogsArgs {
    /// Number of recent lines (bounded to 1000).
    #[arg(long, default_value_t = 100)]
    lines: usize,
}

/// Whether missing first-setup values may be prompted from the terminal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InteractionMode {
    /// stdin is an interactive terminal.
    Interactive,
    /// No prompts are permitted.
    NonInteractive,
}

impl InteractionMode {
    /// Detects terminal interactivity without reading input.
    #[must_use]
    pub fn detect() -> Self {
        if io::stdin().is_terminal() {
            Self::Interactive
        } else {
            Self::NonInteractive
        }
    }
}

/// CLI execution failure after clap parsing.
#[derive(Debug)]
pub enum CliError {
    /// Daemon-owned operation failed.
    Daemon(DaemonError),
    /// Required CLI input is missing or contradictory.
    Usage(String),
    /// JSON projection unexpectedly failed.
    Serialization(String),
    /// Interactive prompt I/O failed.
    Io(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Daemon(error) => error.fmt(formatter),
            Self::Usage(detail) => write!(formatter, "invalid command: {detail}"),
            Self::Serialization(detail) => write!(formatter, "unable to render JSON: {detail}"),
            Self::Io(detail) => write!(formatter, "terminal input failed: {detail}"),
        }
    }
}

impl std::error::Error for CliError {}

impl From<DaemonError> for CliError {
    fn from(error: DaemonError) -> Self {
        Self::Daemon(error)
    }
}

/// Executes one parsed command against an injected daemon-owned runtime.
pub async fn execute(
    cli: Cli,
    runtime: &LocalRuntime,
    interaction: InteractionMode,
) -> Result<String, CliError> {
    if cli.internal_daemon {
        return Err(CliError::Usage(
            "--internal-daemon is reserved for the detached child entry".to_owned(),
        ));
    }
    match cli.command {
        Some(Command::Setup(arguments)) => setup(runtime, arguments, interaction).await,
        Some(Command::Status(arguments)) => status(runtime, arguments.json).await,
        Some(Command::Doctor(arguments)) => doctor(runtime, arguments.json).await,
        Some(Command::Daemon { command }) => match command {
            DaemonCommand::Status(arguments) => status(runtime, arguments.json).await,
            DaemonCommand::Stop(arguments) => stop(runtime, arguments.force).await,
            DaemonCommand::Restart(arguments) => restart(runtime, arguments.force).await,
        },
        Some(Command::Logs(arguments)) => logs(runtime, arguments.lines),
        None => {
            let mut command = Cli::command();
            Ok(format!("{}\n", command.render_long_help()))
        }
    }
}

async fn setup(
    runtime: &LocalRuntime,
    mut arguments: SetupArgs,
    interaction: InteractionMode,
) -> Result<String, CliError> {
    let observed = runtime.observe().await?;
    if arguments.name.is_none() && arguments.profile.is_none() && arguments.relay_url.is_none() {
        match &observed {
            ObservedState::Running(status) => return Ok(render_setup_status(status)),
            ObservedState::ConfiguredStopped(setup) => {
                runtime.ensure().await?;
                return Ok(render_setup_result(setup));
            }
            ObservedState::NotConfigured => {}
        }
    }

    apply_committed_defaults(&observed, &mut arguments);
    let name = required_or_prompt(
        arguments.name,
        "--name <name>",
        "Device name: ",
        interaction,
    )?;
    let profile = match arguments.profile {
        Some(profile) => profile.as_str().to_owned(),
        None if interaction == InteractionMode::Interactive => {
            let value = prompt("Infrastructure profile [official-n0]: ")?;
            if value.trim().is_empty() {
                "official-n0".to_owned()
            } else {
                value.trim().to_owned()
            }
        }
        None => {
            return Err(CliError::Usage(
                "first noninteractive setup requires --profile <official-n0|self-hosted>"
                    .to_owned(),
            ));
        }
    };
    let relay_url = if profile == "self-hosted" && arguments.relay_url.is_none() {
        Some(required_or_prompt(
            None,
            "--relay-url <https-url>",
            "Self-hosted Relay URL: ",
            interaction,
        )?)
    } else {
        arguments.relay_url
    };
    let requested = validate_setup_profile(&name, &profile, relay_url.as_deref())?;
    let result = runtime.setup(&requested).await?;
    Ok(render_setup_result(&result))
}

fn apply_committed_defaults(observed: &ObservedState, arguments: &mut SetupArgs) {
    match observed {
        ObservedState::ConfiguredStopped(setup) => {
            if arguments.name.is_none() {
                arguments.name = Some(setup.config.device_name.clone());
            }
            if arguments.profile.is_none() {
                arguments.profile = Some(match setup.config.infrastructure.profile_name() {
                    "self-hosted" => ProfileArg::SelfHosted,
                    _ => ProfileArg::OfficialN0,
                });
            }
            if arguments.relay_url.is_none() {
                arguments.relay_url = setup
                    .config
                    .infrastructure
                    .relay_url()
                    .map(ToString::to_string);
            }
        }
        ObservedState::Running(status) => {
            if arguments.name.is_none() {
                arguments.name = Some(status.device_name.clone());
            }
            if arguments.profile.is_none() {
                arguments.profile = Some(if status.infrastructure_profile == "self-hosted" {
                    ProfileArg::SelfHosted
                } else {
                    ProfileArg::OfficialN0
                });
            }
        }
        ObservedState::NotConfigured => {}
    }
}

fn required_or_prompt(
    value: Option<String>,
    flag: &str,
    prompt_text: &str,
    interaction: InteractionMode,
) -> Result<String, CliError> {
    if let Some(value) = value {
        return Ok(value);
    }
    if interaction == InteractionMode::NonInteractive {
        return Err(CliError::Usage(format!(
            "first noninteractive setup requires {flag}"
        )));
    }
    let value = prompt(prompt_text)?;
    if value.trim().is_empty() {
        Err(CliError::Usage(format!("{flag} cannot be empty")))
    } else {
        Ok(value.trim().to_owned())
    }
}

fn prompt(text: &str) -> Result<String, CliError> {
    print!("{text}");
    io::stdout()
        .flush()
        .map_err(|error| CliError::Io(error.to_string()))?;
    let mut value = String::new();
    io::stdin()
        .read_line(&mut value)
        .map_err(|error| CliError::Io(error.to_string()))?;
    Ok(value)
}

async fn status(runtime: &LocalRuntime, json: bool) -> Result<String, CliError> {
    let view = StatusView::from_observed(runtime.observe().await?);
    if json {
        json_line(&view)
    } else {
        Ok(view.human())
    }
}

async fn doctor(runtime: &LocalRuntime, json: bool) -> Result<String, CliError> {
    let report = runtime.doctor().await;
    if json {
        let view = DoctorView::from(report);
        json_line(&view)
    } else {
        let mut output = String::new();
        for check in report.checks {
            let marker = if check.ok { "ok" } else { "error" };
            output.push_str(&format!("[{marker}] {}: {}\n", check.name, check.detail));
        }
        Ok(output)
    }
}

async fn stop(runtime: &LocalRuntime, force: bool) -> Result<String, CliError> {
    Ok(match runtime.stop(force).await? {
        Some(impact) => format!(
            "Daemon stopping ({} active sessions).\n",
            impact.active_session_count
        ),
        None => "Daemon already stopped.\n".to_owned(),
    })
}

async fn restart(runtime: &LocalRuntime, force: bool) -> Result<String, CliError> {
    let readiness = runtime.restart(force).await?;
    Ok(format!(
        "Daemon ready (zterm {}, wire {}).\n",
        readiness.version, readiness.protocol.wire_major
    ))
}

fn logs(runtime: &LocalRuntime, lines: usize) -> Result<String, CliError> {
    let mut output = format!("{}\n", runtime.daemon_log_path().display());
    for line in runtime.log_tail(lines)? {
        output.push_str(&line);
        output.push('\n');
    }
    Ok(output)
}

fn render_setup_result(result: &BootstrapResult) -> String {
    format!(
        "Configured {}.\nDevice ID: {}\nDaemon: running\n",
        result.config.device_name, result.endpoint_id
    )
}

fn render_setup_status(status: &DaemonStatus) -> String {
    format!(
        "Configured {}.\nDevice ID: {}\nDaemon: running\n",
        status.device_name, status.endpoint_id
    )
}

#[derive(Serialize)]
struct StatusView {
    state: &'static str,
    version: Option<String>,
    phase: Option<String>,
    endpoint_id: Option<String>,
    device_name: Option<String>,
    infrastructure_profile: Option<String>,
    started_at_unix: Option<u64>,
    active_session_count: u32,
    active_session_names: Vec<String>,
}

impl StatusView {
    fn from_observed(observed: ObservedState) -> Self {
        match observed {
            ObservedState::Running(status) => Self {
                state: "running",
                version: Some(status.version),
                phase: Some(status.phase),
                endpoint_id: Some(status.endpoint_id),
                device_name: Some(status.device_name),
                infrastructure_profile: Some(status.infrastructure_profile),
                started_at_unix: Some(status.started_at_unix),
                active_session_count: status.active_session_count,
                active_session_names: status.active_session_names,
            },
            ObservedState::ConfiguredStopped(setup) => Self {
                state: "configured_stopped",
                version: Some(env!("CARGO_PKG_VERSION").to_owned()),
                phase: Some(zterm_core::PHASE_NAME.to_owned()),
                endpoint_id: Some(setup.endpoint_id),
                device_name: Some(setup.config.device_name),
                infrastructure_profile: Some(setup.config.infrastructure.profile_name().to_owned()),
                started_at_unix: None,
                active_session_count: 0,
                active_session_names: Vec::new(),
            },
            ObservedState::NotConfigured => Self {
                state: "not_configured",
                version: Some(env!("CARGO_PKG_VERSION").to_owned()),
                phase: Some(zterm_core::PHASE_NAME.to_owned()),
                endpoint_id: None,
                device_name: None,
                infrastructure_profile: None,
                started_at_unix: None,
                active_session_count: 0,
                active_session_names: Vec::new(),
            },
        }
    }

    fn human(&self) -> String {
        let mut output = format!("State: {}\n", self.state);
        if let Some(name) = &self.device_name {
            output.push_str(&format!("Device: {name}\n"));
        }
        if let Some(endpoint) = &self.endpoint_id {
            output.push_str(&format!("Device ID: {endpoint}\n"));
        }
        if let Some(profile) = &self.infrastructure_profile {
            output.push_str(&format!("Infrastructure: {profile}\n"));
        }
        output.push_str(&format!("Active sessions: {}\n", self.active_session_count));
        output
    }
}

#[derive(Serialize)]
struct DoctorView {
    healthy: bool,
    checks: Vec<DoctorCheckView>,
}

#[derive(Serialize)]
struct DoctorCheckView {
    name: &'static str,
    ok: bool,
    detail: String,
}

impl From<DoctorReport> for DoctorView {
    fn from(report: DoctorReport) -> Self {
        Self {
            healthy: report.healthy,
            checks: report
                .checks
                .into_iter()
                .map(|check| DoctorCheckView {
                    name: check.name,
                    ok: check.ok,
                    detail: check.detail,
                })
                .collect(),
        }
    }
}

fn json_line(value: &impl Serialize) -> Result<String, CliError> {
    serde_json::to_string_pretty(value)
        .map(|json| format!("{json}\n"))
        .map_err(|error| CliError::Serialization(error.to_string()))
}
