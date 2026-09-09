//! Frontend-owned Session protocol, transport, and typed terminal view.
//!
//! The daemon server owns host Sessions and opaque remote tunnels. This boundary
//! owns frontend attachment epochs, correlation, recovery, and control budgets.

pub mod ipc;
#[cfg(unix)]
mod session;
#[cfg(unix)]
mod transport;
pub mod view;

#[cfg(unix)]
pub use ipc::LocalPairingClient;
pub use ipc::{LocalClient, LocalDeviceClient};
#[cfg(unix)]
pub use session::{LocalAttachmentEvent, LocalTakeoverRetryToken, SessionClient};
#[cfg(unix)]
pub(crate) use transport::{RemoteDaemonRestarter, UnixAttachmentConnector};

/// Creates a desktop upload connector using the existing opaque daemon tunnel.
#[cfg(unix)]
pub fn upload_connector(
    socket: impl Into<std::path::PathBuf>,
) -> std::sync::Arc<dyn zterm_client::upload::UploadConnector> {
    std::sync::Arc::new(transport::UnixAttachmentConnector::new(socket))
}

#[cfg(unix)]
pub(super) use zterm_client::protocol::{
    decode_response, malformed, resource_error, service_error,
};

#[cfg(unix)]
pub(super) use zterm_client::protocol::DEFAULT_DEADLINE;

#[cfg(unix)]
use transport::{connect_error, daemon_io};
