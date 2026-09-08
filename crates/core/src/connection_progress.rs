//! Content-free observations of one terminal connection attempt.

/// A real operation boundary; events do not imply that another stage succeeded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionStage {
    /// The interactive command has started.
    Starting,
    /// Physical terminal setup is starting.
    InitializingTerminal,
    /// Physical setup and bounded color observation finished.
    TerminalInitialized,
    /// Checking or starting the configured local daemon.
    CheckingLocalService,
    /// The configured local daemon is ready.
    LocalServiceReady,
    /// Resolving the user's immutable target.
    ResolvingTarget,
    /// The target was resolved.
    TargetResolved,
    /// Opening the local IPC socket.
    OpeningLocalChannel,
    /// Asking the local daemon for a remote tunnel.
    OpeningRemoteChannel,
    /// Checking the existing peer connection.
    CheckingConnection,
    /// An authenticated primary was already available.
    ReusingConnection,
    /// Resolving routes for an actual dial attempt.
    LookingUpAddress,
    /// Establishing an authenticated transport.
    ConnectingSecurely,
    /// Transport identity verification succeeded.
    SecureConnectionReady,
    /// Performing the normal application handshake.
    CheckingProtocolAndAccess,
    /// The normal handshake confirmed compatibility and remote authorization.
    ProtocolAndAccessReady,
    /// Waiting for deterministic connection selection.
    SelectingConnection,
    /// Opening the selected connection's service stream.
    OpeningSessionChannel,
    /// The service channel is ready.
    SessionChannelReady,
    /// Another real connection attempt will run.
    RetryingConnection,
    /// Creating an explicitly named Session.
    CreatingSession,
    /// The named Session was created.
    SessionCreated,
    /// Sending the Session attach request.
    RequestingSession,
    /// Waiting for the authoritative initial snapshot.
    ReceivingTerminalSnapshot,
    /// A valid initial snapshot arrived.
    TerminalSnapshotReceived,
    /// Displaying the first validated terminal frame.
    DisplayingTerminal,
    /// Acknowledging the displayed initial state.
    SynchronizingTerminal,
    /// The frontend input fence is open.
    TerminalReady,
    /// Cancellation is waiting for an already-submitted operation.
    Cancelling,
    /// The Session ended before the frontend input fence opened.
    SessionEnded,
    /// Connection startup was cancelled.
    Cancelled,
    /// Connection startup failed.
    Failed,
}

impl ConnectionStage {
    /// Stable file-log code and English display text, without untrusted data.
    #[must_use]
    pub const fn description(self) -> (&'static str, &'static str) {
        match self {
            Self::Starting => ("starting", "Starting ZTerm"),
            Self::InitializingTerminal => ("initializing_terminal", "Initializing terminal"),
            Self::TerminalInitialized => ("terminal_initialized", "Terminal initialized"),
            Self::CheckingLocalService => ("checking_local_service", "Checking local service"),
            Self::LocalServiceReady => ("local_service_ready", "Local service ready"),
            Self::ResolvingTarget => ("resolving_target", "Resolving target device"),
            Self::TargetResolved => ("target_resolved", "Target device resolved"),
            Self::OpeningLocalChannel => ("opening_local_channel", "Opening local channel"),
            Self::OpeningRemoteChannel => ("opening_remote_channel", "Requesting remote channel"),
            Self::CheckingConnection => {
                ("checking_connection", "Checking for an existing connection")
            }
            Self::ReusingConnection => ("reusing_connection", "Reusing authenticated connection"),
            Self::LookingUpAddress => ("looking_up_address", "Looking up device address"),
            Self::ConnectingSecurely => ("connecting_securely", "Establishing secure connection"),
            Self::SecureConnectionReady => {
                ("secure_connection_ready", "Secure connection established")
            }
            Self::CheckingProtocolAndAccess => (
                "checking_protocol_and_access",
                "Checking protocol and access",
            ),
            Self::ProtocolAndAccessReady => {
                ("protocol_and_access_ready", "Protocol and access confirmed")
            }
            Self::SelectingConnection => ("selecting_connection", "Selecting usable connection"),
            Self::OpeningSessionChannel => ("opening_session_channel", "Opening session channel"),
            Self::SessionChannelReady => ("session_channel_ready", "Session channel ready"),
            Self::RetryingConnection => ("retrying_connection", "Retrying connection"),
            Self::CreatingSession => ("creating_session", "Creating session"),
            Self::SessionCreated => ("session_created", "Session created"),
            Self::RequestingSession => ("requesting_session", "Requesting session"),
            Self::ReceivingTerminalSnapshot => {
                ("receiving_terminal_state", "Receiving terminal state")
            }
            Self::TerminalSnapshotReceived => {
                ("terminal_state_received", "Terminal state received")
            }
            Self::DisplayingTerminal => ("displaying_terminal", "Displaying terminal"),
            Self::SynchronizingTerminal => ("synchronizing_terminal", "Synchronizing terminal"),
            Self::TerminalReady => ("terminal_ready", "Terminal ready"),
            Self::Cancelling => ("cancelling", "Cancelling; waiting for session result"),
            Self::SessionEnded => (
                "session_ended",
                "Session ended before terminal became ready",
            ),
            Self::Cancelled => ("cancelled", "Connection cancelled"),
            Self::Failed => ("failed", "Connection failed"),
        }
    }
}
