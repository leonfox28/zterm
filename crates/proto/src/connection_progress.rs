use crate::{ProtocolError, v2};
use zterm_core::connection_progress::ConnectionStage;

/// Encodes only the stages owned by the local connection broker.
pub fn connection_stage_to_message(stage: ConnectionStage) -> Option<v2::LocalConnectionProgress> {
    let stage = match stage {
        ConnectionStage::CheckingConnection => v2::LocalConnectionStage::CheckingConnection,
        ConnectionStage::ReusingConnection => v2::LocalConnectionStage::ReusingConnection,
        ConnectionStage::LookingUpAddress => v2::LocalConnectionStage::LookingUpAddress,
        ConnectionStage::ConnectingSecurely => v2::LocalConnectionStage::ConnectingSecurely,
        ConnectionStage::SecureConnectionReady => v2::LocalConnectionStage::SecureConnectionReady,
        ConnectionStage::CheckingProtocolAndAccess => {
            v2::LocalConnectionStage::CheckingProtocolAndAccess
        }
        ConnectionStage::ProtocolAndAccessReady => v2::LocalConnectionStage::ProtocolAndAccessReady,
        ConnectionStage::SelectingConnection => v2::LocalConnectionStage::SelectingConnection,
        ConnectionStage::OpeningSessionChannel => v2::LocalConnectionStage::OpeningSessionChannel,
        ConnectionStage::SessionChannelReady => v2::LocalConnectionStage::SessionChannelReady,
        ConnectionStage::RetryingConnection => v2::LocalConnectionStage::RetryingConnection,
        _ => return None,
    };
    Some(v2::LocalConnectionProgress {
        stage: stage as i32,
    })
}

/// Rejects unspecified/unknown local stages before they reach presentation.
pub fn connection_stage_from_message(
    message: v2::LocalConnectionProgress,
) -> Result<ConnectionStage, ProtocolError> {
    let invalid = || ProtocolError::InvalidLocalConnectionStage;
    Ok(
        match v2::LocalConnectionStage::try_from(message.stage).map_err(|_| invalid())? {
            v2::LocalConnectionStage::CheckingConnection => ConnectionStage::CheckingConnection,
            v2::LocalConnectionStage::ReusingConnection => ConnectionStage::ReusingConnection,
            v2::LocalConnectionStage::LookingUpAddress => ConnectionStage::LookingUpAddress,
            v2::LocalConnectionStage::ConnectingSecurely => ConnectionStage::ConnectingSecurely,
            v2::LocalConnectionStage::SecureConnectionReady => {
                ConnectionStage::SecureConnectionReady
            }
            v2::LocalConnectionStage::CheckingProtocolAndAccess => {
                ConnectionStage::CheckingProtocolAndAccess
            }
            v2::LocalConnectionStage::ProtocolAndAccessReady => {
                ConnectionStage::ProtocolAndAccessReady
            }
            v2::LocalConnectionStage::SelectingConnection => ConnectionStage::SelectingConnection,
            v2::LocalConnectionStage::OpeningSessionChannel => {
                ConnectionStage::OpeningSessionChannel
            }
            v2::LocalConnectionStage::SessionChannelReady => ConnectionStage::SessionChannelReady,
            v2::LocalConnectionStage::RetryingConnection => ConnectionStage::RetryingConnection,
            v2::LocalConnectionStage::Unspecified => return Err(invalid()),
        },
    )
}
