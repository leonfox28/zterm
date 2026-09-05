//! Thin command parsing and rendering over daemon-owned operations.

use std::fmt;
use std::io::{self, IsTerminal, Read, Write};
use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use unicode_width::UnicodeWidthStr;
use zeroize::Zeroizing;
use zterm_core::{AuthorizationStatus, MAX_TICKET_TEXT_BYTES, SessionId, validate_pair_ttl};
use zterm_daemon::bootstrap::BootstrapResult;
use zterm_daemon::config::validate_setup_profile;
use zterm_daemon::error::DaemonError;
use zterm_daemon::operations::{
    CommandDeviceSummary, CommandSessionSummary, LocalRuntime, ObservedState, UpdateStage,
};
use zterm_daemon::pairing::PairTicketText;
use zterm_daemon::service::{DaemonStatus, SessionImpact};

mod terminal_ui;

pub use terminal_ui::run_terminal;

const SETUP_GUIDANCE: &str = "zterm is not configured. Run `zterm setup` first.\n";

/// zterm's public command tree plus one hidden daemon entry flag.
#[derive(Parser)]
#[command(
    name = "zterm",
    version,
    about = "Secure persistent terminal sessions across trusted devices",
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Internal detached daemon entry; never accepted as a state-path override.
    #[arg(
        long,
        hide = true,
        conflicts_with_all = ["internal_release_self_check", "internal_release_verify", "internal_release_install"]
    )]
    internal_daemon: bool,
    /// Internal side-effect-free build identity used by the installer.
    #[arg(
        long,
        hide = true,
        conflicts_with_all = ["internal_daemon", "internal_release_verify", "internal_release_install"]
    )]
    internal_release_self_check: bool,
    /// Internal exact manifest/signature verification used by the installer.
    #[arg(
        long,
        hide = true,
        num_args = 2,
        value_names = ["MANIFEST", "SIGNATURE"],
        conflicts_with_all = ["internal_daemon", "internal_release_self_check", "internal_release_install"]
    )]
    internal_release_verify: Option<Vec<PathBuf>>,
    /// Internal no-clobber installation of this already verified candidate.
    #[arg(
        long,
        hide = true,
        value_name = "DESTINATION",
        conflicts_with_all = ["internal_daemon", "internal_release_self_check", "internal_release_verify"]
    )]
    internal_release_install: Option<PathBuf>,
    /// Public operation to perform.
    #[command(subcommand)]
    command: Option<Command>,
}

impl fmt::Debug for Cli {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Cli")
            .field("internal_daemon", &self.internal_daemon)
            .field(
                "internal_release_self_check",
                &self.internal_release_self_check,
            )
            .field(
                "internal_release_verify_present",
                &self.internal_release_verify.is_some(),
            )
            .field(
                "internal_release_install_present",
                &self.internal_release_install.is_some(),
            )
            .field("command", &self.command)
            .finish()
    }
}

impl Cli {
    /// Whether this parse selected the hidden pre-runtime daemon entry.
    #[must_use]
    pub const fn internal_daemon(&self) -> bool {
        self.internal_daemon
    }

    /// Whether this parse selected the side-effect-free release identity entry.
    #[must_use]
    pub const fn internal_release_self_check(&self) -> bool {
        self.internal_release_self_check
    }

    /// Exact manifest/signature paths selected by the hidden verifier entry.
    #[must_use]
    pub fn internal_release_verify(&self) -> Option<(&std::path::Path, &std::path::Path)> {
        match self.internal_release_verify.as_deref() {
            Some([manifest, signature]) => Some((manifest.as_path(), signature.as_path())),
            _ => None,
        }
    }

    /// Exact destination selected by the hidden candidate installer entry.
    #[must_use]
    pub fn internal_release_install(&self) -> Option<&std::path::Path> {
        self.internal_release_install.as_deref()
    }

    /// Whether any hidden pre-runtime entry was selected.
    #[must_use]
    pub fn has_internal_entry(&self) -> bool {
        self.internal_daemon
            || self.internal_release_self_check
            || self.internal_release_verify.is_some()
            || self.internal_release_install.is_some()
    }
}

/// Public commands backed by the same-UID local daemon.
#[derive(Debug, Subcommand)]
enum Command {
    /// Configure this device and explicitly start its daemon.
    Setup(SetupArgs),
    /// Show setup and daemon state without starting anything.
    Status,
    /// Run local-only diagnostics without starting anything.
    Doctor,
    /// Create or accept one-time device pairing tickets.
    Pair {
        /// Pairing operation.
        #[command(subcommand)]
        command: PairCommand,
    },
    /// Inspect or manage directional trusted-device records.
    Device {
        /// Device operation.
        #[command(subcommand)]
        command: DeviceCommand,
    },
    /// Attach the default or selected persistent Session.
    Connect(ConnectArgs),
    /// Inspect or manage persistent Sessions.
    Session {
        /// Session operation.
        #[command(subcommand)]
        command: SessionCommand,
    },
    /// Inspect or control the per-user daemon.
    Daemon {
        /// Daemon lifecycle operation.
        #[command(subcommand)]
        command: DaemonCommand,
    },
    /// Print a bounded recent daemon log tail without starting anything.
    Logs(LogsArgs),
    /// Destroy this host's managed identity and pairing state.
    Reset(ResetArgs),
    /// Explicitly download, verify, and install a newer official Release.
    Update(UpdateArgs),
    /// Remove the complete managed state and this installed executable.
    Uninstall(UninstallArgs),
}

#[derive(clap::Args)]
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

impl fmt::Debug for SetupArgs {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SetupArgs")
            .field("name", &self.name)
            .field("profile", &self.profile)
            .field("relay_url", &"[REDACTED]")
            .field("relay_url_present", &self.relay_url.is_some())
            .finish()
    }
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

#[derive(Debug, Subcommand)]
enum PairCommand {
    /// Create a bounded one-time bearer ticket.
    Create(PairCreateArgs),
    /// Accept a bearer ticket from a no-echo TTY or explicit stdin automation.
    Accept(PairAcceptArgs),
}

#[derive(Debug, clap::Args)]
struct PairCreateArgs {
    /// Ticket lifetime such as 60s, 10m, or 1h.
    #[arg(long, value_parser = parse_pair_ttl)]
    ttl: Option<u32>,
}

#[derive(Debug, clap::Args)]
struct PairAcceptArgs {
    /// Read the ticket from stdin for explicit automation instead of a TTY.
    #[arg(long)]
    stdin: bool,
    /// Exact outbound alias assigned after acceptance.
    #[arg(long)]
    alias: Option<String>,
}

#[derive(Debug, Subcommand)]
enum DeviceCommand {
    /// List outbound-known and inbound-authorization directions.
    List,
    /// Rename only the outbound known-device alias.
    Rename(DeviceRenameArgs),
    /// Revoke only the remote device's inbound authorization.
    Revoke(DeviceRevokeArgs),
}

#[derive(Debug, clap::Args)]
struct DeviceRenameArgs {
    /// Exact alias or canonical full Device ID.
    device: String,
    /// New exact outbound alias.
    alias: String,
}

#[derive(Debug, clap::Args)]
struct DeviceRevokeArgs {
    /// Exact alias or canonical full Device ID.
    device: String,
    /// Confirm without an interactive prompt.
    #[arg(short = 'y', long)]
    yes: bool,
}

#[derive(Debug, clap::Args)]
struct ConnectArgs {
    /// Exact outbound device alias/full ID, or the reserved local target.
    target: String,
    /// Exact Session name or canonical full Session ID.
    #[arg(long, default_value = "main")]
    session: String,
    /// Explicitly replace an existing controller after synchronization.
    #[arg(long)]
    takeover: bool,
    /// Local detach prefix: ctrl-@ through ctrl-_, ctrl-?, or none.
    #[arg(long, default_value = "ctrl-]", value_parser = parse_escape_prefix)]
    escape: EscapePrefix,
}

#[derive(Debug, Subcommand)]
enum SessionCommand {
    /// List live Sessions on one exact target.
    List(SessionListArgs),
    /// Create a named Session, then attach that exact created identity.
    New(SessionNewArgs),
    /// Attach one exact existing Session.
    Attach(SessionAttachArgs),
    /// Rename one exact existing Session.
    Rename(SessionRenameArgs),
    /// Explicitly close one exact existing Session.
    Close(SessionCloseArgs),
}

#[derive(Debug, clap::Args)]
struct SessionListArgs {
    /// Exact outbound device alias/full ID, or local (the default).
    #[arg(default_value = "local")]
    target: String,
}

#[derive(clap::Args)]
struct SessionNewArgs {
    /// Exact outbound device alias/full ID, or local.
    target: String,
    /// Exact new Session name.
    name: String,
    /// Working directory interpreted by the selected host.
    #[arg(long)]
    cwd: Option<PathBuf>,
    /// Local detach prefix: ctrl-@ through ctrl-_, ctrl-?, or none.
    #[arg(long, default_value = "ctrl-]", value_parser = parse_escape_prefix)]
    escape: EscapePrefix,
}

impl fmt::Debug for SessionNewArgs {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionNewArgs")
            .field("target", &self.target)
            .field("name", &self.name)
            .field("cwd", &"[REDACTED]")
            .field("cwd_present", &self.cwd.is_some())
            .field("escape", &self.escape)
            .finish()
    }
}

#[derive(Debug, clap::Args)]
struct SessionAttachArgs {
    /// Exact outbound device alias/full ID, or local.
    target: String,
    /// Exact Session name or canonical full Session ID.
    session: String,
    /// Explicitly replace an existing controller after synchronization.
    #[arg(long)]
    takeover: bool,
    /// Local detach prefix: ctrl-@ through ctrl-_, ctrl-?, or none.
    #[arg(long, default_value = "ctrl-]", value_parser = parse_escape_prefix)]
    escape: EscapePrefix,
}

#[derive(Debug, clap::Args)]
struct SessionRenameArgs {
    /// Exact outbound device alias/full ID, or local.
    target: String,
    /// Exact Session name or canonical full Session ID.
    session: String,
    /// Exact new Session name.
    new_name: String,
}

#[derive(Debug, clap::Args)]
struct SessionCloseArgs {
    /// Exact outbound device alias/full ID, or local.
    target: String,
    /// Exact Session name or canonical full Session ID.
    session: String,
    /// Confirm without an interactive prompt.
    #[arg(short = 'y', long)]
    yes: bool,
}

#[derive(Debug, Subcommand)]
enum DaemonCommand {
    /// Show daemon state without starting it.
    Status,
    /// Gracefully stop the daemon; already stopped succeeds.
    Stop(YesArgs),
    /// Gracefully stop and explicitly start one daemon.
    Restart(YesArgs),
}

#[derive(Debug, clap::Args)]
struct YesArgs {
    /// Confirm without prompting; end active Sessions and their PTYs.
    #[arg(short = 'y', long)]
    yes: bool,
}

#[derive(Debug, clap::Args)]
struct LogsArgs {
    /// Number of recent lines (bounded to 1000).
    #[arg(short = 'n', long, default_value_t = 100)]
    lines: usize,
}

#[derive(Debug, clap::Args)]
struct ResetArgs {
    /// Destroy this host's exact managed identity and all pairing state.
    #[arg(long, required = true)]
    identity: bool,
    /// Confirm without an interactive prompt.
    #[arg(short = 'y', long)]
    yes: bool,
}

#[derive(Debug, clap::Args)]
struct UpdateArgs {
    /// Install one exact published stable or prerelease tag instead of latest stable.
    #[arg(long, value_name = "TAG")]
    version: Option<String>,
    /// Confirm without prompting; end active Sessions after candidate verification.
    #[arg(short = 'y', long)]
    yes: bool,
}

#[derive(Debug, clap::Args)]
struct UninstallArgs {
    /// Confirm without an interactive prompt.
    #[arg(short = 'y', long)]
    yes: bool,
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
pub enum CliError {
    /// Daemon-owned operation failed.
    Daemon(DaemonError),
    /// Required CLI input is missing or contradictory.
    Usage(String),
    /// Interactive prompt I/O failed.
    Io(String),
    /// A Session was created successfully, but its follow-up attach failed.
    CreatedSessionAttach {
        /// Stable identity which remains live after the attach failure.
        session_id: SessionId,
        /// Typed attach failure.
        source: DaemonError,
    },
    /// The daemon-owned retained terminal driver failed.
    TerminalDriverFailure,
}

impl fmt::Debug for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Daemon(error) => formatter
                .debug_struct("Daemon")
                .field("error_kind", &error.kind())
                .finish(),
            Self::Usage(detail) => formatter
                .debug_struct("Usage")
                .field("detail", &"[REDACTED]")
                .field("detail_len", &detail.len())
                .finish(),
            Self::Io(detail) => formatter
                .debug_struct("Io")
                .field("detail", &"[REDACTED]")
                .field("detail_len", &detail.len())
                .finish(),
            Self::CreatedSessionAttach { session_id, source } => formatter
                .debug_struct("CreatedSessionAttach")
                .field("session_id", session_id)
                .field("error_kind", &source.kind())
                .finish(),
            Self::TerminalDriverFailure => formatter.write_str("TerminalDriverFailure"),
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Daemon(error) => error.fmt(formatter),
            Self::Usage(detail) => write!(formatter, "invalid command: {detail}"),
            Self::Io(detail) => write!(formatter, "interactive terminal failed: {detail}"),
            Self::CreatedSessionAttach { session_id, source } => write!(
                formatter,
                "session {session_id} was created and remains live, but attach failed: {source}"
            ),
            Self::TerminalDriverFailure => {
                formatter.write_str("the daemon-owned terminal driver failed")
            }
        }
    }
}

impl std::error::Error for CliError {}

impl From<DaemonError> for CliError {
    fn from(error: DaemonError) -> Self {
        Self::Daemon(error)
    }
}

/// Per-invocation local control-prefix selection.
#[derive(Clone, Copy, Eq, PartialEq)]
struct EscapePrefix(Option<u8>);

impl EscapePrefix {
    const DEFAULT: Self = Self(Some(0x1d));
}

impl fmt::Debug for EscapePrefix {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(byte) => formatter
                .debug_tuple("EscapePrefix")
                .field(&format_args!("control-{byte:#04x}"))
                .finish(),
            None => formatter.write_str("EscapePrefix(None)"),
        }
    }
}

#[cfg(unix)]
enum TerminalRequestKind {
    Attach {
        target: String,
        selector: Option<String>,
        create_main: bool,
        takeover: bool,
    },
    Create {
        target: String,
        name: String,
        working_directory: Option<PathBuf>,
    },
}

#[cfg(not(unix))]
enum TerminalRequestKind {
    Attach,
    Create,
}

/// Deferred terminal operation. It owns no attachment, socket, frame, or route.
pub struct TerminalRequest {
    kind: TerminalRequestKind,
    escape: EscapePrefix,
}

impl fmt::Debug for TerminalRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let operation = match self.kind {
            #[cfg(unix)]
            TerminalRequestKind::Attach { .. } => "attach",
            #[cfg(not(unix))]
            TerminalRequestKind::Attach => "attach",
            #[cfg(unix)]
            TerminalRequestKind::Create { .. } => "create-and-attach",
            #[cfg(not(unix))]
            TerminalRequestKind::Create => "create-and-attach",
        };
        formatter
            .debug_struct("TerminalRequest")
            .field("operation", &operation)
            .field("escape", &self.escape)
            .finish_non_exhaustive()
    }
}

/// Typed command result. Terminal operations remain deferred until TTY preflight.
pub enum CommandOutcome {
    /// Ordinary non-secret command output.
    Text(String),
    /// The only secret-bearing stdout projection, zeroized after it is written.
    PairTicket(Zeroizing<String>),
    /// Deferred raw-terminal operation containing no attachment or transport owner.
    Terminal(TerminalRequest),
}

impl fmt::Debug for CommandOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(text) => formatter
                .debug_struct("Text")
                .field("text", &"[REDACTED]")
                .field("text_len", &text.len())
                .finish(),
            Self::PairTicket(_) => formatter.write_str("PairTicket([REDACTED])"),
            Self::Terminal(request) => formatter.debug_tuple("Terminal").field(request).finish(),
        }
    }
}

impl CommandOutcome {
    /// Extracts ordinary text for deterministic command tests.
    pub fn into_text(self) -> Result<String, CliError> {
        match self {
            Self::Text(text) => Ok(text),
            Self::PairTicket(_) => Err(CliError::Usage(
                "pair-ticket output must be written through its zeroizing owner".to_owned(),
            )),
            Self::Terminal(_) => Err(CliError::Usage(
                "terminal requests must be run through the raw-terminal driver".to_owned(),
            )),
        }
    }
}

/// Executes one parsed command against an injected daemon-owned runtime.
pub async fn execute(
    cli: Cli,
    runtime: &LocalRuntime,
    interaction: InteractionMode,
) -> Result<CommandOutcome, CliError> {
    if cli.has_internal_entry() {
        return Err(CliError::Usage(
            "internal entries are reserved for detached daemon and release verification".to_owned(),
        ));
    }
    match cli.command {
        Some(Command::Setup(arguments)) => setup(runtime, arguments, interaction)
            .await
            .map(CommandOutcome::Text),
        Some(Command::Status) => status(runtime).await.map(CommandOutcome::Text),
        Some(Command::Doctor) => doctor(runtime).await.map(CommandOutcome::Text),
        Some(Command::Pair { command }) => pair(runtime, command, interaction).await,
        Some(Command::Device { command }) => device(runtime, command, interaction).await,
        Some(Command::Connect(arguments)) => connect(runtime, arguments).await,
        Some(Command::Session { command }) => session(runtime, command, interaction).await,
        Some(Command::Daemon { command }) => match command {
            DaemonCommand::Status => status(runtime).await.map(CommandOutcome::Text),
            DaemonCommand::Stop(arguments) => stop(runtime, arguments.yes, interaction)
                .await
                .map(CommandOutcome::Text),
            DaemonCommand::Restart(arguments) => restart(runtime, arguments.yes, interaction)
                .await
                .map(CommandOutcome::Text),
        },
        Some(Command::Logs(arguments)) => logs(runtime, arguments.lines).map(CommandOutcome::Text),
        Some(Command::Reset(arguments)) => reset(runtime, arguments, interaction).await,
        Some(Command::Update(arguments)) => update(runtime, arguments, interaction).await,
        Some(Command::Uninstall(arguments)) => uninstall(runtime, arguments, interaction).await,
        None => bare(runtime).await,
    }
}

async fn bare(runtime: &LocalRuntime) -> Result<CommandOutcome, CliError> {
    match runtime.observe().await? {
        ObservedState::NotConfigured => Ok(CommandOutcome::Text(SETUP_GUIDANCE.to_owned())),
        ObservedState::Running(_) | ObservedState::ConfiguredStopped(_) => {
            connect(
                runtime,
                ConnectArgs {
                    target: "local".to_owned(),
                    session: "main".to_owned(),
                    takeover: false,
                    escape: EscapePrefix::DEFAULT,
                },
            )
            .await
        }
    }
}

async fn pair(
    runtime: &LocalRuntime,
    command: PairCommand,
    interaction: InteractionMode,
) -> Result<CommandOutcome, CliError> {
    match command {
        PairCommand::Create(arguments) => {
            let ttl = arguments.ttl.unwrap_or(
                u32::try_from(zterm_core::DEFAULT_PAIR_TTL_SECONDS)
                    .expect("default TTL fits wire field"),
            );
            let ticket = runtime.pair_create(ttl).await?;
            eprintln!(
                "Ticket expires in {ttl} seconds. On the connecting device, run zterm pair accept and paste this ticket."
            );
            let output = Zeroizing::new(format!("{}\n", ticket.expose()));
            drop(ticket);
            Ok(CommandOutcome::PairTicket(output))
        }
        PairCommand::Accept(arguments) => {
            runtime.ensure_configured_daemon().await?;
            let ticket = read_pair_ticket(arguments.stdin, interaction)?;
            let device = runtime
                .pair_accept(ticket, arguments.alias.as_deref())
                .await?;
            let alias = device
                .alias
                .clone()
                .unwrap_or_else(|| device.device_id.to_string());
            Ok(CommandOutcome::Text(format!(
                "Paired as {alias}. This device can now connect to that host.\nConnect with: zterm connect {}\n",
                connect_target(&alias)
            )))
        }
    }
}

async fn device(
    runtime: &LocalRuntime,
    command: DeviceCommand,
    interaction: InteractionMode,
) -> Result<CommandOutcome, CliError> {
    match command {
        DeviceCommand::List => runtime
            .device_list()
            .await
            .map_err(Into::into)
            .map(render_devices)
            .map(CommandOutcome::Text),
        DeviceCommand::Rename(arguments) => {
            let device = runtime
                .device_rename(&arguments.device, &arguments.alias)
                .await?;
            Ok(CommandOutcome::Text(format!(
                "Outbound device {} renamed to {}.\n",
                device.device_id,
                device.alias.as_deref().unwrap_or("(no alias)")
            )))
        }
        DeviceCommand::Revoke(arguments) => {
            let selected = runtime.device_resolve(&arguments.device).await?;
            if selected.inbound_status == AuthorizationStatus::None {
                return Err(CliError::Usage(
                    "the selected device has no inbound authorization to revoke".to_owned(),
                ));
            }
            confirm(
                &format!(
                    "Revoke inbound control from {}? Outbound-known={} and its Sessions remain unchanged.",
                    selected.device_id, selected.outbound_known
                ),
                arguments.yes,
                interaction,
            )?;
            let revoked = runtime.device_revoke(selected.device_id).await?;
            Ok(CommandOutcome::Text(format!(
                "Inbound authorization revoked for {}. Outbound-known={} remains unchanged.\n",
                revoked.device_id, revoked.outbound_known
            )))
        }
    }
}

async fn connect(
    _runtime: &LocalRuntime,
    arguments: ConnectArgs,
) -> Result<CommandOutcome, CliError> {
    let escape = arguments.escape;
    #[cfg(unix)]
    let kind = {
        let create_main = arguments.session == "main";
        let selector = (!create_main).then_some(arguments.session);
        TerminalRequestKind::Attach {
            target: arguments.target,
            selector,
            create_main,
            takeover: arguments.takeover,
        }
    };
    #[cfg(not(unix))]
    let kind = {
        let _ = arguments;
        TerminalRequestKind::Attach
    };
    Ok(CommandOutcome::Terminal(TerminalRequest { kind, escape }))
}

async fn session(
    runtime: &LocalRuntime,
    command: SessionCommand,
    interaction: InteractionMode,
) -> Result<CommandOutcome, CliError> {
    match command {
        SessionCommand::List(arguments) => runtime
            .session_list(&arguments.target)
            .await
            .map_err(Into::into)
            .map(|sessions| render_sessions(sessions, &arguments.target))
            .map(CommandOutcome::Text),
        SessionCommand::New(arguments) => {
            let escape = arguments.escape;
            #[cfg(unix)]
            let kind = TerminalRequestKind::Create {
                target: arguments.target,
                name: arguments.name,
                working_directory: arguments.cwd,
            };
            #[cfg(not(unix))]
            let kind = {
                let _ = arguments;
                TerminalRequestKind::Create
            };
            Ok(CommandOutcome::Terminal(TerminalRequest { kind, escape }))
        }
        SessionCommand::Attach(arguments) => {
            let escape = arguments.escape;
            #[cfg(unix)]
            let kind = TerminalRequestKind::Attach {
                target: arguments.target,
                selector: Some(arguments.session),
                create_main: false,
                takeover: arguments.takeover,
            };
            #[cfg(not(unix))]
            let kind = {
                let _ = arguments;
                TerminalRequestKind::Attach
            };
            Ok(CommandOutcome::Terminal(TerminalRequest { kind, escape }))
        }
        SessionCommand::Rename(arguments) => {
            let renamed = runtime
                .session_rename(&arguments.target, &arguments.session, &arguments.new_name)
                .await?;
            Ok(CommandOutcome::Text(format!(
                "Session {} renamed to {}.\n",
                renamed.session_id, renamed.name
            )))
        }
        SessionCommand::Close(arguments) => {
            let preflight = runtime
                .session_close_preflight(&arguments.target, &arguments.session)
                .await?;
            let selected = preflight.summary();
            let exact_target = preflight
                .target_device_id()
                .map_or_else(|| "local".to_owned(), |device_id| device_id.to_string());
            confirm(
                &format!(
                    "Close Session {} ({}) on target {} and end its PTY?",
                    selected.session_id, selected.name, exact_target
                ),
                arguments.yes,
                interaction,
            )?;
            let closed = runtime.session_close_confirmed(preflight).await?;
            Ok(CommandOutcome::Text(format!(
                "Session {} ({}) closed.\n",
                closed.session_id, closed.name
            )))
        }
    }
}

async fn reset(
    runtime: &LocalRuntime,
    arguments: ResetArgs,
    interaction: InteractionMode,
) -> Result<CommandOutcome, CliError> {
    if !arguments.identity {
        return Err(CliError::Usage(
            "reset requires the explicit --identity boundary".to_owned(),
        ));
    }
    let preflight = runtime.identity_reset_preflight().await?;
    if !preflight.state_present {
        return Ok(CommandOutcome::Text(
            "Managed identity state is already absent. Run `zterm setup` to configure zterm.\n"
                .to_owned(),
        ));
    }
    let public_identity = preflight
        .endpoint_id
        .as_deref()
        .unwrap_or("incomplete identity state");
    confirm(
        &format!(
            "{}Destroy identity {public_identity}, remove all local pairing state, and end all running sessions?",
            session_impact_text(&preflight.active_session_names)
        ),
        arguments.yes,
        interaction,
    )?;
    let result = runtime.reset_identity(preflight.device_id, true).await?;
    Ok(CommandOutcome::Text(if result.removed {
        "Managed identity state removed. Run `zterm setup` to create a new identity.\n".to_owned()
    } else {
        "Managed identity state is already absent. Run `zterm setup` to configure zterm.\n"
            .to_owned()
    }))
}

async fn update(
    runtime: &LocalRuntime,
    arguments: UpdateArgs,
    interaction: InteractionMode,
) -> Result<CommandOutcome, CliError> {
    let result = runtime
        .update_with_callbacks(
            arguments.version.as_deref(),
            arguments.yes,
            |impact| confirm_sessions("Updating zterm", impact, interaction),
            |stage| {
                eprintln!(
                    "{}",
                    match stage {
                        UpdateStage::Preparing => "Downloading and verifying the release...",
                        UpdateStage::Verified => "Release verified.",
                        UpdateStage::Stopping => "Stopping the daemon...",
                        UpdateStage::Activating => "Installing the verified release...",
                        UpdateStage::Starting => "Starting the updated daemon...",
                    }
                )
            },
        )
        .await?;
    let startup = if result.daemon_started {
        "Daemon: running"
    } else {
        "Run zterm setup to configure and start the daemon."
    };
    let ended = if result.ended_session_names.is_empty() {
        String::new()
    } else {
        format!(
            "Ended sessions: {}.\n",
            result.ended_session_names.join(", ")
        )
    };
    Ok(CommandOutcome::Text(format!(
        "Updated zterm from {} to {}.\n{ended}{startup}\n",
        result.previous_version, result.installed_version
    )))
}

async fn uninstall(
    runtime: &LocalRuntime,
    arguments: UninstallArgs,
    interaction: InteractionMode,
) -> Result<CommandOutcome, CliError> {
    let preflight = runtime.uninstall_preflight().await?;
    let identity = preflight
        .identity
        .endpoint_id
        .as_deref()
        .unwrap_or("no committed identity");
    confirm(
        &format!(
            "{}Uninstall zterm {}, destroy identity {}, remove all local pairing state, and end all running sessions? Devices must be paired again after reinstall.",
            session_impact_text(&preflight.identity.active_session_names),
            preflight.version,
            identity
        ),
        arguments.yes,
        interaction,
    )?;
    let result = runtime
        .uninstall(preflight.identity.device_id, true)
        .await?;
    Ok(CommandOutcome::Text(format!(
        "Uninstalled zterm. Managed state removed: {}. Executable removed: {}.\n",
        result.state_removed, result.executable_removed
    )))
}

fn parse_pair_ttl(value: &str) -> Result<u32, String> {
    let (digits, multiplier) = match value.as_bytes().split_last() {
        Some((b's', digits)) => (digits, 1_u64),
        Some((b'm', digits)) => (digits, 60_u64),
        Some((b'h', digits)) => (digits, 60_u64 * 60),
        _ => return Err("TTL must use a checked s, m, or h suffix".to_owned()),
    };
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return Err("TTL must contain only decimal digits before its suffix".to_owned());
    }
    let number = std::str::from_utf8(digits)
        .map_err(|_| "TTL must be ASCII".to_owned())?
        .parse::<u64>()
        .map_err(|_| "TTL value is too large".to_owned())?;
    let seconds = number
        .checked_mul(multiplier)
        .ok_or_else(|| "TTL value is too large".to_owned())?;
    validate_pair_ttl(seconds).map_err(|error| error.to_string())?;
    u32::try_from(seconds).map_err(|_| "TTL value is too large".to_owned())
}

fn parse_escape_prefix(value: &str) -> Result<EscapePrefix, String> {
    if value == "none" {
        return Ok(EscapePrefix(None));
    }
    let suffix = value
        .strip_prefix("ctrl-")
        .ok_or_else(|| "escape must be ctrl-@ through ctrl-_, ctrl-?, or none".to_owned())?;
    let mut characters = suffix.chars();
    let character = characters
        .next()
        .ok_or_else(|| "escape must be ctrl-@ through ctrl-_, ctrl-?, or none".to_owned())?;
    if characters.next().is_some() || !character.is_ascii() {
        return Err("escape must name exactly one ASCII control".to_owned());
    }
    let upper = character.to_ascii_uppercase() as u8;
    let byte = match upper {
        b'@'..=b'_' => upper & 0x1f,
        b'?' => 0x7f,
        _ => {
            return Err("escape must be ctrl-@ through ctrl-_, ctrl-?, or none".to_owned());
        }
    };
    Ok(EscapePrefix(Some(byte)))
}

fn read_pair_ticket(
    explicit_stdin: bool,
    interaction: InteractionMode,
) -> Result<PairTicketText, CliError> {
    if explicit_stdin {
        let stdin = io::stdin();
        let mut reader = stdin.lock();
        return read_pair_ticket_automation(&mut reader);
    }
    if interaction != InteractionMode::Interactive || !io::stdin().is_terminal() {
        return Err(CliError::Usage(
            "pair accept requires a no-echo TTY; automation must opt in with --stdin".to_owned(),
        ));
    }
    #[cfg(unix)]
    {
        print!("Pair ticket: ");
        io::stdout()
            .flush()
            .map_err(|error| CliError::Io(error.to_string()))?;
        let stdin = io::stdin();
        let mut reader = stdin.lock();
        let result = read_pair_ticket_no_echo_from(&stdin, &mut reader);
        println!();
        result
    }
    #[cfg(not(unix))]
    {
        Err(CliError::Usage(
            "no-echo pair ticket input is unavailable on this platform".to_owned(),
        ))
    }
}

fn read_pair_ticket_automation(reader: &mut impl Read) -> Result<PairTicketText, CliError> {
    let mut bytes = Zeroizing::new(Vec::with_capacity(256));
    reader
        .take(u64::try_from(MAX_TICKET_TEXT_BYTES + 1).unwrap_or(u64::MAX))
        .read_to_end(&mut bytes)
        .map_err(|error| CliError::Io(error.to_string()))?;
    if bytes.len() > MAX_TICKET_TEXT_BYTES {
        return Err(CliError::Usage(
            "pair ticket exceeds the 16 KiB input bound".to_owned(),
        ));
    }
    pair_ticket_from_bytes(&bytes)
}

#[cfg(unix)]
fn read_pair_ticket_no_echo_from<T, R>(
    terminal: &T,
    reader: &mut R,
) -> Result<PairTicketText, CliError>
where
    T: std::os::fd::AsFd + ?Sized,
    R: Read,
{
    read_pair_ticket_no_echo_with_observer(terminal, reader, || {})
}

#[cfg(unix)]
fn read_pair_ticket_no_echo_with_observer<T, R, F>(
    terminal: &T,
    reader: &mut R,
    disabled: F,
) -> Result<PairTicketText, CliError>
where
    T: std::os::fd::AsFd + ?Sized,
    R: Read,
    F: FnOnce(),
{
    use nix::sys::termios::{LocalFlags, SetArg, tcgetattr, tcsetattr};

    let original = tcgetattr(terminal).map_err(|error| CliError::Io(error.to_string()))?;
    let mut no_echo = original.clone();
    no_echo.local_flags.remove(LocalFlags::ECHO);
    tcsetattr(terminal, SetArg::TCSANOW, &no_echo)
        .map_err(|error| CliError::Io(error.to_string()))?;
    let guard = EchoGuard {
        terminal,
        original,
        restored: false,
    };
    disabled();

    let result = read_pair_ticket_line(reader);
    guard.restore()?;
    result
}

#[cfg(unix)]
struct EchoGuard<'a, T: std::os::fd::AsFd + ?Sized> {
    terminal: &'a T,
    original: nix::sys::termios::Termios,
    restored: bool,
}

#[cfg(unix)]
impl<T: std::os::fd::AsFd + ?Sized> EchoGuard<'_, T> {
    fn restore(mut self) -> Result<(), CliError> {
        nix::sys::termios::tcsetattr(
            self.terminal,
            nix::sys::termios::SetArg::TCSANOW,
            &self.original,
        )
        .map_err(|error| CliError::Io(error.to_string()))?;
        self.restored = true;
        Ok(())
    }
}

#[cfg(unix)]
impl<T: std::os::fd::AsFd + ?Sized> Drop for EchoGuard<'_, T> {
    fn drop(&mut self) {
        if !self.restored {
            let _ = nix::sys::termios::tcsetattr(
                self.terminal,
                nix::sys::termios::SetArg::TCSANOW,
                &self.original,
            );
        }
    }
}

#[cfg(unix)]
fn read_pair_ticket_line(reader: &mut impl Read) -> Result<PairTicketText, CliError> {
    let mut bytes = Zeroizing::new(Vec::with_capacity(256));
    loop {
        let mut byte = [0_u8; 1];
        match reader.read(&mut byte) {
            Ok(0) => break,
            Ok(_) if byte[0] == b'\n' || byte[0] == b'\r' => break,
            Ok(_) => {
                if bytes.len() == MAX_TICKET_TEXT_BYTES {
                    return Err(CliError::Usage(
                        "pair ticket exceeds the 16 KiB input bound".to_owned(),
                    ));
                }
                bytes.push(byte[0]);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(CliError::Io(error.to_string())),
        }
    }
    pair_ticket_from_bytes(&bytes)
}

fn pair_ticket_from_bytes(bytes: &[u8]) -> Result<PairTicketText, CliError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| CliError::Usage("pair ticket must be UTF-8 text".to_owned()))?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(CliError::Usage("pair ticket input is empty".to_owned()));
    }
    PairTicketText::from_local_response(trimmed.to_owned())
        .map_err(|_| CliError::Usage("pair ticket is invalid or expired".to_owned()))
}

fn confirm(impact: &str, yes: bool, interaction: InteractionMode) -> Result<(), CliError> {
    confirm_with(impact, yes, interaction, || {
        prompt(&format!("{impact} Continue? [y/N]: "))
    })
}

fn confirm_with(
    impact: &str,
    yes: bool,
    interaction: InteractionMode,
    read: impl FnOnce() -> Result<String, CliError>,
) -> Result<(), CliError> {
    if yes {
        return Ok(());
    }
    if interaction == InteractionMode::NonInteractive {
        return Err(CliError::Usage(format!(
            "{impact} Run again with -y to continue without prompting."
        )));
    }
    let answer = read()?;
    if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        Err(CliError::Daemon(DaemonError::new(
            zterm_core::DomainErrorKind::Cancelled,
            "Confirmation cancelled.",
        )))
    }
}

fn session_impact_text(names: &[String]) -> String {
    if names.is_empty() {
        return String::new();
    }
    format!(
        "The following sessions are running:\n  {}\n\n",
        names.join("\n  ")
    )
}

fn confirm_sessions(
    action: &str,
    impact: &SessionImpact,
    interaction: InteractionMode,
) -> Result<(), DaemonError> {
    confirm(
        &format!(
            "{}{action} will end all running sessions.",
            session_impact_text(&impact.active_session_names)
        ),
        false,
        interaction,
    )
    .map_err(|error| DaemonError::new(zterm_core::DomainErrorKind::Cancelled, error.to_string()))
}

fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-./".contains(&byte))
    {
        value.to_owned()
    } else {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
}

fn connect_target(value: &str) -> String {
    let quoted = shell_quote(value);
    if value.starts_with('-') {
        format!("-- {quoted}")
    } else {
        quoted
    }
}

fn padded(value: &str, width: usize) -> String {
    format!("{value}{}", " ".repeat(width.saturating_sub(value.width())))
}

fn authorization_status(status: AuthorizationStatus) -> &'static str {
    match status {
        AuthorizationStatus::None => "None",
        AuthorizationStatus::Authorized => "Allowed",
        AuthorizationStatus::Revoked => "Revoked",
    }
}

fn render_devices(devices: Vec<CommandDeviceSummary>) -> String {
    if devices.is_empty() {
        return "No paired devices. Run zterm pair create to allow another device to connect, or zterm pair accept to connect to a host.\n".to_owned();
    }
    fn name(device: &CommandDeviceSummary) -> &str {
        device
            .alias
            .as_deref()
            .or(device.remote_name.as_deref())
            .unwrap_or("(unnamed)")
    }
    let width = devices
        .iter()
        .map(|device| name(device).width())
        .max()
        .unwrap_or(4)
        .max(4);
    let mut output = format!(
        "{}  {}  {}  Inbound control\n",
        padded("Name", width),
        padded("Connection", 13),
        padded("Connect to host", 15)
    );
    for device in &devices {
        output.push_str(&format!(
            "{}  {}  {}  {}\n  ID: {}\n",
            padded(name(device), width),
            padded(
                if device.online {
                    "Connected"
                } else {
                    "Not connected"
                },
                13
            ),
            padded(
                if device.outbound_known {
                    "Available"
                } else {
                    "Not paired"
                },
                15
            ),
            authorization_status(device.inbound_status),
            device.device_id
        ));
    }
    output
}

fn render_sessions(sessions: Vec<CommandSessionSummary>, target: &str) -> String {
    if sessions.is_empty() {
        return format!(
            "No running sessions. Run zterm connect {} to start the main session.\n",
            connect_target(target)
        );
    }
    let width = sessions
        .iter()
        .map(|session| session.name.as_str().width())
        .max()
        .unwrap_or(4)
        .max(4);
    let mut output = format!(
        "{}  {}  Viewport\n",
        padded("Name", width),
        padded("Controller", 10)
    );
    for session in sessions {
        output.push_str(&format!(
            "{}  {}  {}x{}\n  ID: {}\n",
            padded(session.name.as_str(), width),
            padded(
                if session.has_controller {
                    "Attached"
                } else {
                    "Detached"
                },
                10
            ),
            session.viewport.columns,
            session.viewport.rows,
            session.session_id
        ));
    }
    output
}

async fn setup(
    runtime: &LocalRuntime,
    mut arguments: SetupArgs,
    interaction: InteractionMode,
) -> Result<String, CliError> {
    let observed = runtime.observe().await?;
    // A running self-hosted daemon deliberately does not disclose its Relay
    // URL in status. Reusing its exact existing setup needs no URL or prompt.
    if let ObservedState::Running(status) = &observed
        && arguments.relay_url.is_none()
        && arguments
            .name
            .as_deref()
            .is_none_or(|name| name == status.device_name)
        && arguments
            .profile
            .is_none_or(|profile| profile.as_str() == status.infrastructure_profile)
    {
        return Ok(render_setup_status(status));
    }
    if arguments.name.is_none() && arguments.profile.is_none() && arguments.relay_url.is_none() {
        match &observed {
            ObservedState::Running(status) => return Ok(render_setup_status(status)),
            ObservedState::ConfiguredStopped(setup) => {
                runtime.ensure_configured_daemon().await?;
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
    let profile = arguments.profile.unwrap_or(ProfileArg::OfficialN0).as_str();
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
    let requested = validate_setup_profile(&name, profile, relay_url.as_deref())?;
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

async fn status(runtime: &LocalRuntime) -> Result<String, CliError> {
    Ok(StatusView::from_observed(runtime.observe().await?).human())
}

async fn doctor(runtime: &LocalRuntime) -> Result<String, CliError> {
    let report = runtime.doctor().await;
    let mut output = String::new();
    for check in report.checks {
        let marker = if check.ok { "ok" } else { "error" };
        output.push_str(&format!("[{marker}] {}: {}\n", check.name, check.detail));
    }
    if let Ok(observed) = runtime.observe().await {
        output.push_str(&StatusView::from_observed(observed).diagnostics());
    }
    Ok(output)
}

async fn stop(
    runtime: &LocalRuntime,
    yes: bool,
    interaction: InteractionMode,
) -> Result<String, CliError> {
    Ok(
        match runtime
            .stop_with_confirmation(yes, |impact| {
                confirm_sessions("Stopping the daemon", impact, interaction)
            })
            .await?
        {
            Some(impact) => format!(
                "Daemon stopped ({} sessions ended).\n",
                impact.active_session_count
            ),
            None => "Daemon already stopped.\n".to_owned(),
        },
    )
}

async fn restart(
    runtime: &LocalRuntime,
    yes: bool,
    interaction: InteractionMode,
) -> Result<String, CliError> {
    let readiness = runtime
        .restart_with_confirmation(yes, |impact| {
            confirm_sessions("Restarting the daemon", impact, interaction)
        })
        .await?;
    Ok(format!(
        "Daemon ready (zterm {}, wire {}).\n",
        readiness.version, readiness.protocol.wire_major
    ))
}

fn logs(runtime: &LocalRuntime, lines: usize) -> Result<String, CliError> {
    let mut output = String::new();
    for line in runtime.log_tail(lines)? {
        output.push_str(&line);
        output.push('\n');
    }
    if output.is_empty() && lines > 0 {
        output.push_str("No daemon logs yet.\n");
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
    network_state: Option<String>,
    endpoint_bound: Option<bool>,
    network_bind_attempts: u64,
    address_publish_state: Option<String>,
    address_lookup_state: Option<String>,
    authenticated_connection_count: u32,
    primary_connection_count: u32,
    active_stream_count: u32,
    direct_path_count: u32,
    relay_path_count: u32,
    network_diagnostic: Option<String>,
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
                network_state: Some(status.network.state.as_str().to_owned()),
                endpoint_bound: Some(status.network.endpoint_bound),
                network_bind_attempts: status.network.bind_attempts,
                address_publish_state: Some(status.network.publish.as_str().to_owned()),
                address_lookup_state: Some(status.network.lookup.as_str().to_owned()),
                authenticated_connection_count: status.network.authenticated_connection_count,
                primary_connection_count: status.network.primary_connection_count,
                active_stream_count: status.network.active_stream_count,
                direct_path_count: status.network.direct_path_count,
                relay_path_count: status.network.relay_path_count,
                network_diagnostic: status
                    .network
                    .diagnostic
                    .map(|diagnostic| diagnostic.code().to_owned()),
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
                network_state: Some("stopped".to_owned()),
                endpoint_bound: Some(false),
                network_bind_attempts: 0,
                address_publish_state: Some("disabled".to_owned()),
                address_lookup_state: Some("disabled".to_owned()),
                authenticated_connection_count: 0,
                primary_connection_count: 0,
                active_stream_count: 0,
                direct_path_count: 0,
                relay_path_count: 0,
                network_diagnostic: None,
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
                network_state: None,
                endpoint_bound: None,
                network_bind_attempts: 0,
                address_publish_state: None,
                address_lookup_state: None,
                authenticated_connection_count: 0,
                primary_connection_count: 0,
                active_stream_count: 0,
                direct_path_count: 0,
                relay_path_count: 0,
                network_diagnostic: None,
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
        if let Some(version) = &self.version {
            output.push_str(&format!("Version: {version}\n"));
        }
        output.push_str(&format!(
            "Daemon: {}\n",
            if self.state == "running" {
                "running"
            } else {
                "stopped"
            }
        ));
        if let Some(network) = &self.network_state {
            output.push_str(&format!("Network: {network}\n"));
        }
        output.push_str(&format!("Active sessions: {}\n", self.active_session_count));
        for name in &self.active_session_names {
            output.push_str(&format!("  {name}\n"));
        }
        if self.state == "not_configured" {
            output.push_str(SETUP_GUIDANCE);
        }
        output
    }

    fn diagnostics(&self) -> String {
        let mut output = String::new();
        if let Some(phase) = &self.phase {
            output.push_str(&format!("Build phase: {phase}\n"));
        }
        if let Some(started) = self.started_at_unix {
            output.push_str(&format!("Started at (Unix): {started}\n"));
        }
        if let Some(network) = &self.network_state {
            output.push_str(&format!("Network: {network}\n"));
            if let Some(bound) = self.endpoint_bound {
                output.push_str(&format!("Endpoint bound: {bound}\n"));
            }
            output.push_str(&format!(
                "Network bind attempts: {}\n",
                self.network_bind_attempts
            ));
            if let Some(publish) = &self.address_publish_state {
                output.push_str(&format!("Address publish: {publish}\n"));
            }
            if let Some(lookup) = &self.address_lookup_state {
                output.push_str(&format!("Address lookup: {lookup}\n"));
            }
        }
        if self.network_state.is_some() {
            output.push_str(&format!(
                "Connections: authenticated={}, primary={}, streams={}\n",
                self.authenticated_connection_count,
                self.primary_connection_count,
                self.active_stream_count,
            ));
            output.push_str(&format!(
                "Paths: direct={}, relay={}\n",
                self.direct_path_count, self.relay_path_count,
            ));
            if let Some(diagnostic) = &self.network_diagnostic {
                output.push_str(&format!("Network diagnostic: {diagnostic}\n"));
            }
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use zterm_core::terminal::TerminalSize;
    use zterm_core::{AuthGeneration, DeviceId, Revision, SessionName};
    use zterm_daemon::network::{
        AddressServiceState, NetworkDiagnostic, NetworkObservation, NetworkState,
    };
    use zterm_daemon::service::ProtocolStatus;
    use zterm_daemon::session::SessionSummary as DaemonSessionSummary;

    use super::*;

    #[test]
    fn terminal_io_error_uses_a_transport_neutral_diagnostic() {
        let error = CliError::Io("restore terminal attributes: operation failed".to_owned());

        assert_eq!(
            error.to_string(),
            "interactive terminal failed: restore terminal attributes: operation failed"
        );
    }

    #[test]
    fn status_overview_and_doctor_preserve_redacted_network_details() {
        let device_id = DeviceId::from_array([0x63; 32]);
        let view = StatusView::from_observed(ObservedState::Running(DaemonStatus {
            protocol: ProtocolStatus {
                wire_major: zterm_core::WIRE_MAJOR,
                state_schema: 1,
                capabilities: zterm_core::Capabilities::LOCAL_LIFECYCLE,
            },
            version: "test".to_owned(),
            phase: "test".to_owned(),
            device_id,
            endpoint_id: "public-endpoint".to_owned(),
            device_name: "status-host".to_owned(),
            infrastructure_profile: "official-n0".to_owned(),
            started_at_unix: 1,
            active_session_count: 2,
            active_session_names: vec!["one".to_owned(), "two".to_owned()],
            network: NetworkObservation {
                device_id,
                state: NetworkState::Degraded,
                endpoint_bound: true,
                bind_attempts: 7,
                home_relay: Some("https://relay.example.test".to_owned()),
                publish: AddressServiceState::Configured,
                lookup: AddressServiceState::Degraded,
                authenticated_connection_count: 4,
                primary_connection_count: 2,
                active_stream_count: 5,
                direct_path_count: 1,
                relay_path_count: 1,
                diagnostic: Some(NetworkDiagnostic::HomeRelayUnavailable),
            },
        }));

        let human = view.human();
        assert!(human.contains("Network: degraded"));
        assert!(human.contains("Version: test"));
        assert!(human.contains("  one\n  two"));
        assert!(!human.contains("Network bind attempts"));
        let diagnostics = view.diagnostics();
        assert!(diagnostics.contains("Network bind attempts: 7"));
        assert!(diagnostics.contains("Address publish: configured"));
        assert!(diagnostics.contains("Address lookup: degraded"));
        assert!(diagnostics.contains("Connections: authenticated=4, primary=2, streams=5"));
        assert!(diagnostics.contains("Paths: direct=1, relay=1"));
        assert!(diagnostics.contains("Network diagnostic: home_relay_unavailable"));

        let rendered = format!("{human}{diagnostics}");
        for forbidden in [
            "direct_ip",
            "route_cache",
            "pair_secret",
            "ticket",
            "relay.example.test",
        ] {
            assert!(!rendered.contains(forbidden));
        }
    }

    #[test]
    fn confirmation_accepts_y_and_yes_and_otherwise_cancels_without_unneeded_reads() {
        for answer in ["y", " Y ", "yes", "YeS\n"] {
            confirm_with("End sessions?", false, InteractionMode::Interactive, || {
                Ok(answer.to_owned())
            })
            .expect("accepted confirmation");
        }
        for answer in ["", "\n", "n", "false", "anything"] {
            assert!(
                confirm_with("End sessions?", false, InteractionMode::Interactive, || Ok(
                    answer.to_owned()
                ))
                .is_err()
            );
        }
        confirm_with(
            "End sessions?",
            true,
            InteractionMode::NonInteractive,
            || panic!("must not read"),
        )
        .expect("accepted confirmation");
        assert!(
            confirm_with(
                "End sessions?",
                false,
                InteractionMode::NonInteractive,
                || panic!("must not read")
            )
            .expect_err("unconfirmed noninteractive invocation")
            .to_string()
            .contains("-y")
        );
        assert_eq!(shell_quote("team's host"), "'team'\"'\"'s host'");
        assert_eq!(connect_target("-host"), "-- -host");
        assert_eq!(padded("开发", 6), "开发  ");
        assert!(render_sessions(Vec::new(), "local").contains("zterm connect local"));
        assert!(render_devices(Vec::new()).contains("zterm pair"));
    }

    #[test]
    fn public_parser_accepts_human_conveniences_and_rejects_removed_options() {
        for args in [
            vec!["status", "--json"],
            vec!["doctor", "--json"],
            vec!["daemon", "status", "--json"],
            vec!["device", "list", "--json"],
            vec!["session", "list", "--json"],
            vec!["daemon", "stop", "--force"],
            vec!["daemon", "restart", "--force"],
            vec!["update", "--force"],
            vec!["uninstall", "--force"],
            vec!["reset", "--identity", "--force"],
            vec!["pair", "accept", "--name", "host"],
            vec!["logs", "-f"],
        ] {
            assert!(Cli::try_parse_from(std::iter::once("zterm").chain(args)).is_err());
        }
        for args in [
            vec!["daemon", "stop", "-y"],
            vec!["daemon", "restart", "-y"],
            vec!["update", "-y"],
            vec!["uninstall", "-y"],
            vec!["reset", "--identity", "-y"],
            vec!["device", "revoke", "host", "-y"],
            vec!["session", "close", "local", "main", "-y"],
            vec!["pair", "accept", "--alias", "host"],
            vec!["logs", "-n", "20"],
            vec!["session", "list"],
        ] {
            assert!(Cli::try_parse_from(std::iter::once("zterm").chain(args)).is_ok());
        }
    }

    #[test]
    fn pair_ttl_parser_owns_suffix_and_product_bounds() {
        assert_eq!(parse_pair_ttl("60s"), Ok(60));
        assert_eq!(parse_pair_ttl("10m"), Ok(600));
        assert_eq!(parse_pair_ttl("1h"), Ok(3_600));
        for invalid in ["600", "0s", "1d", "-10m", "999999999999999999999h"] {
            assert!(parse_pair_ttl(invalid).is_err(), "accepted {invalid}");
        }
    }

    #[test]
    fn escape_parser_supports_default_custom_delete_and_disabled_controls() {
        assert_eq!(parse_escape_prefix("ctrl-]"), Ok(EscapePrefix(Some(0x1d))));
        assert_eq!(parse_escape_prefix("ctrl-@"), Ok(EscapePrefix(Some(0x00))));
        assert_eq!(parse_escape_prefix("ctrl-a"), Ok(EscapePrefix(Some(0x01))));
        assert_eq!(parse_escape_prefix("ctrl-?"), Ok(EscapePrefix(Some(0x7f))));
        assert_eq!(parse_escape_prefix("none"), Ok(EscapePrefix(None)));
        for invalid in ["ctrl-a-b", "a", "CTRL-A", "ctrl-é"] {
            assert!(parse_escape_prefix(invalid).is_err(), "accepted {invalid}");
        }
    }

    #[test]
    fn device_table_preserves_directions_without_route_data() {
        let outbound = CommandDeviceSummary {
            device_id: DeviceId::from_array([0x71; DeviceId::LENGTH]),
            outbound_known: true,
            alias: Some("laptop".to_owned()),
            remote_name: Some("Laptop".to_owned()),
            inbound_status: AuthorizationStatus::None,
            generation: AuthGeneration::ZERO,
            paired_at_unix: 0,
            last_seen_at_unix: 10,
            online: true,
            active_stream_count: 2,
            remote_attachment_count: 0,
        };
        let inbound = CommandDeviceSummary {
            device_id: DeviceId::from_array([0x72; DeviceId::LENGTH]),
            outbound_known: false,
            alias: None,
            remote_name: None,
            inbound_status: AuthorizationStatus::Authorized,
            generation: AuthGeneration::new(4).expect("nonzero generation"),
            paired_at_unix: 20,
            last_seen_at_unix: 30,
            online: false,
            active_stream_count: 0,
            remote_attachment_count: 1,
        };

        let human = render_devices(vec![outbound.clone(), inbound.clone()]);
        assert!(human.contains("Name") && human.contains("Inbound control"));
        assert!(
            human.contains("Available")
                && human.contains("Allowed")
                && human.contains("Not paired")
        );
        assert!(human.contains("Not connected") && !human.contains("Offline"));
        assert!(
            human.contains(&outbound.device_id.to_string())
                && human.contains(&inbound.device_id.to_string())
        );
        for forbidden in ["route", "relay", "ticket", "working_directory"] {
            assert!(!human.contains(forbidden));
        }
    }

    #[test]
    fn session_table_keeps_identity_without_sensitive_diagnostics() {
        let cwd_sentinel = "/private/tmp/CLI_CWD_SENTINEL_8eb1/project";
        let forbidden_sentinels = [
            cwd_sentinel,
            "https://CLI_ROUTE_SENTINEL_452d.example.test/private",
            "198.51.100.207:43210",
            "CLI_TICKET_SENTINEL_539a",
            "CLI_PROOF_SENTINEL_d10f",
            "CLI_KEY_SENTINEL_82c6",
            "CLI_TERMINAL_SENTINEL_0ad4",
            "CLI_INPUT_SENTINEL_b63e",
        ];
        let daemon_summary = DaemonSessionSummary {
            session_id: SessionId::from_array([0x74; SessionId::LENGTH]),
            name: SessionName::new("cli-safe-session").expect("valid session name"),
            revision: Revision::new(67),
            has_controller: true,
            working_directory: cwd_sentinel.into(),
            viewport: TerminalSize::new(53, 179),
        };
        let command_summary = CommandSessionSummary::from(daemon_summary);

        let rendered = render_sessions(vec![command_summary.clone()], "local");
        assert!(rendered.contains("cli-safe-session"));
        assert!(rendered.contains(&command_summary.session_id.to_string()));
        assert!(rendered.contains("Attached") && rendered.contains("179x53"));
        assert!(!rendered.contains("revision"));
        for sentinel in forbidden_sentinels {
            assert!(!rendered.contains(sentinel));
        }
        for forbidden_key in ["working_directory", "route", "direct_ip", "secret"] {
            assert!(!rendered.contains(forbidden_key));
        }
    }

    #[test]
    fn parsed_command_debug_redacts_relay_url_and_working_directory_wrappers() {
        let relay_sentinel = "https://CLI_DEBUG_ROUTE_SENTINEL_5f7a.example.test/private";
        let cwd_sentinel = "/private/tmp/CLI_DEBUG_CWD_SENTINEL_b145/project";
        let setup = Cli::try_parse_from([
            "zterm",
            "setup",
            "--name",
            "setup-debug-host",
            "--profile",
            "self-hosted",
            "--relay-url",
            relay_sentinel,
        ])
        .expect("setup command parses");
        let session = Cli::try_parse_from([
            "zterm",
            "session",
            "new",
            "local",
            "cli-debug-session",
            "--cwd",
            cwd_sentinel,
        ])
        .expect("Session command parses");

        let rendered = format!("{setup:?} {session:?}");
        assert!(!rendered.contains(relay_sentinel));
        assert!(!rendered.contains(cwd_sentinel));
        assert!(rendered.contains("[REDACTED]"));
        assert!(rendered.contains("relay_url_present: true"));
        assert!(rendered.contains("cwd_present: true"));
        assert!(rendered.contains("setup-debug-host"));
        assert!(rendered.contains("cli-debug-session"));
    }

    #[test]
    fn ticket_parse_errors_and_debug_never_echo_input() {
        let no_tty = read_pair_ticket(false, InteractionMode::NonInteractive)
            .expect_err("implicit piped input is rejected without reading");
        assert!(no_tty.to_string().contains("--stdin"));

        let secret = b"private-invalid-zterm-ticket";
        let error = pair_ticket_from_bytes(secret).expect_err("invalid ticket rejected");
        assert!(!error.to_string().contains("private-invalid"));
        assert!(!format!("{error:?}").contains("private-invalid"));

        let outcome =
            CommandOutcome::PairTicket(Zeroizing::new("private-ticket-output\n".to_owned()));
        assert_eq!(format!("{outcome:?}"), "PairTicket([REDACTED])");

        const TEXT_SENTINEL: &str = "/private/tmp/COMMAND_TEXT_SENTINEL_93de";
        let text = CommandOutcome::Text(TEXT_SENTINEL.to_owned());
        let rendered = format!("{text:?}");
        assert!(!rendered.contains(TEXT_SENTINEL));
        assert!(rendered.contains("text: \"[REDACTED]\""));
        assert!(rendered.contains(&format!("text_len: {}", TEXT_SENTINEL.len())));
        assert_eq!(
            text.into_text()
                .expect("ordinary output remains accessible"),
            TEXT_SENTINEL
        );

        const ERROR_SENTINEL: &str = "/private/tmp/CLI_ERROR_SENTINEL_f247";
        let daemon = CliError::Daemon(DaemonError::new(
            zterm_core::DomainErrorKind::PathUnsafe,
            ERROR_SENTINEL,
        ));
        let usage = CliError::Usage(ERROR_SENTINEL.to_owned());
        let io = CliError::Io(ERROR_SENTINEL.to_owned());
        let created = CliError::CreatedSessionAttach {
            session_id: SessionId::from_array([0x91; SessionId::LENGTH]),
            source: DaemonError::new(zterm_core::DomainErrorKind::PathUnsafe, ERROR_SENTINEL),
        };
        let rendered = format!("{daemon:?} {usage:?} {io:?} {created:?}");
        assert!(!rendered.contains(ERROR_SENTINEL));
        assert!(rendered.contains("error_kind: PathUnsafe"));
        assert!(rendered.contains("detail: \"[REDACTED]\""));
        assert_eq!(
            CliError::Usage("ordinary usage detail".to_owned()).to_string(),
            "invalid command: ordinary usage detail"
        );
    }

    #[cfg(unix)]
    #[test]
    fn no_echo_ticket_reader_restores_echo_after_error() {
        use std::fs::File;
        use std::os::fd::AsFd;
        use std::sync::mpsc;

        use nix::pty::openpty;
        use nix::sys::termios::{LocalFlags, tcgetattr};

        let pty = openpty(None, None).expect("open test PTY");
        let mut master = File::from(pty.master);
        let mut slave_reader = File::from(pty.slave);
        let slave_terminal = slave_reader.try_clone().expect("terminal control clone");
        let slave_probe = slave_reader.try_clone().expect("termios probe clone");
        assert!(
            tcgetattr(slave_probe.as_fd())
                .expect("initial termios")
                .local_flags
                .contains(LocalFlags::ECHO)
        );

        let (disabled_tx, disabled_rx) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            read_pair_ticket_no_echo_with_observer(&slave_terminal, &mut slave_reader, || {
                disabled_tx.send(()).expect("publish no-echo boundary")
            })
        });
        disabled_rx
            .recv()
            .expect("reader reached the no-echo boundary");
        assert!(
            !tcgetattr(slave_probe.as_fd())
                .expect("disabled termios")
                .local_flags
                .contains(LocalFlags::ECHO)
        );
        master
            .write_all(b"private-invalid-ticket\n")
            .expect("write ticket fixture");
        let error = reader
            .join()
            .expect("ticket reader thread")
            .expect_err("invalid fixture remains invalid");
        assert!(!error.to_string().contains("private-invalid"));
        assert!(
            tcgetattr(slave_probe.as_fd())
                .expect("restored termios")
                .local_flags
                .contains(LocalFlags::ECHO)
        );
    }

    #[cfg(unix)]
    #[test]
    fn no_echo_ticket_reader_restores_echo_after_success() {
        use std::fs::File;
        use std::os::fd::AsFd;
        use std::sync::mpsc;

        use nix::pty::openpty;
        use nix::sys::termios::{LocalFlags, tcgetattr};

        let ticket = valid_pair_ticket();
        let pty = openpty(None, None).expect("open test PTY");
        let mut master = File::from(pty.master);
        let mut slave_reader = File::from(pty.slave);
        let slave_terminal = slave_reader.try_clone().expect("terminal control clone");
        let slave_probe = slave_reader.try_clone().expect("termios probe clone");
        let (disabled_tx, disabled_rx) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            read_pair_ticket_no_echo_with_observer(&slave_terminal, &mut slave_reader, || {
                disabled_tx.send(()).expect("publish no-echo boundary")
            })
        });
        disabled_rx
            .recv()
            .expect("reader reached the no-echo boundary");
        master
            .write_all(ticket.expose().as_bytes())
            .and_then(|()| master.write_all(b"\n"))
            .expect("write valid ticket fixture");

        let parsed = reader
            .join()
            .expect("ticket reader thread")
            .expect("valid ticket");
        assert_eq!(parsed.expose(), ticket.expose());
        assert!(
            tcgetattr(slave_probe.as_fd())
                .expect("restored termios")
                .local_flags
                .contains(LocalFlags::ECHO)
        );
    }

    #[cfg(unix)]
    #[test]
    fn no_echo_ticket_reader_restores_echo_during_panic_unwind() {
        use std::fs::File;
        use std::os::fd::AsFd;
        use std::panic::{AssertUnwindSafe, catch_unwind};
        use std::sync::mpsc;

        use nix::pty::openpty;
        use nix::sys::termios::{LocalFlags, tcgetattr};

        let pty = openpty(None, None).expect("open test PTY");
        let _master = File::from(pty.master);
        let mut slave_reader = File::from(pty.slave);
        let slave_terminal = slave_reader.try_clone().expect("terminal control clone");
        let slave_probe = slave_reader.try_clone().expect("termios probe clone");
        let (disabled_tx, disabled_rx) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            catch_unwind(AssertUnwindSafe(|| {
                read_pair_ticket_no_echo_with_observer(&slave_terminal, &mut slave_reader, || {
                    disabled_tx.send(()).expect("publish no-echo boundary");
                    panic!("injected ticket-reader panic");
                })
            }))
        });
        disabled_rx
            .recv()
            .expect("reader reached the no-echo boundary");

        assert!(reader.join().expect("ticket reader thread").is_err());
        assert!(
            tcgetattr(slave_probe.as_fd())
                .expect("restored termios")
                .local_flags
                .contains(LocalFlags::ECHO)
        );
    }

    #[cfg(unix)]
    fn valid_pair_ticket() -> PairTicketText {
        use zterm_core::{
            DeviceDisplayName, EphemeralOperationId, PairFingerprint, RelayHint, TransportLimits,
        };
        use zterm_daemon::pairing::{PairOfferRequest, PairingManager};

        let ttl_seconds = 60;
        let manager = PairingManager::new(
            DeviceId::from_array([0x51; DeviceId::LENGTH]),
            TransportLimits::default(),
        )
        .expect("pure pairing manager");
        manager
            .create_offer(
                PairOfferRequest::new(
                    EphemeralOperationId::from_array([0x52; 16]),
                    PairFingerprint::for_create(ttl_seconds),
                    DeviceDisplayName::new("cli-echo-test").expect("display name"),
                    vec![RelayHint::new("https://relay.example.test").expect("relay hint")],
                    ttl_seconds,
                )
                .expect("pair offer request"),
            )
            .expect("pair offer")
            .ticket()
            .clone()
    }
}
