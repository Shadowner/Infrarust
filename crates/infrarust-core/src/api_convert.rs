pub const fn convert_server_state(
    sm: infrarust_server_manager::ServerState,
) -> infrarust_api::services::server_manager::ServerState {
    use infrarust_api::services::server_manager::ServerState as ApiState;
    use infrarust_server_manager::ServerState as SmState;

    match sm {
        SmState::Online => ApiState::Online,
        SmState::Sleeping => ApiState::Sleeping,
        SmState::Starting => ApiState::Starting,
        SmState::Stopping => ApiState::Stopping,
        SmState::Crashed => ApiState::Crashed,
        _ => ApiState::Offline,
    }
}

pub fn protocol_state_to_api(
    state: infrarust_protocol::version::ConnectionState,
) -> infrarust_api::event::ConnectionState {
    match state {
        infrarust_protocol::version::ConnectionState::Handshake => {
            infrarust_api::event::ConnectionState::Handshake
        }
        infrarust_protocol::version::ConnectionState::Status => {
            infrarust_api::event::ConnectionState::Status
        }
        infrarust_protocol::version::ConnectionState::Login => {
            infrarust_api::event::ConnectionState::Login
        }
        infrarust_protocol::version::ConnectionState::Config => {
            infrarust_api::event::ConnectionState::Configuration
        }
        infrarust_protocol::version::ConnectionState::Play => {
            infrarust_api::event::ConnectionState::Play
        }
    }
}

pub fn protocol_direction_to_api(
    direction: infrarust_protocol::version::Direction,
) -> infrarust_api::events::packet::PacketDirection {
    match direction {
        infrarust_protocol::version::Direction::Serverbound => {
            infrarust_api::events::packet::PacketDirection::Serverbound
        }
        infrarust_protocol::version::Direction::Clientbound => {
            infrarust_api::events::packet::PacketDirection::Clientbound
        }
    }
}

pub fn pack_result_to_api(
    result: infrarust_protocol::packets::resource_pack::ResourcePackResult,
) -> infrarust_api::player::ResourcePackStatus {
    use infrarust_api::player::ResourcePackStatus as Api;
    use infrarust_protocol::packets::resource_pack::ResourcePackResult as Protocol;

    match result {
        Protocol::SuccessfullyLoaded => Api::SuccessfullyLoaded,
        Protocol::Declined => Api::Declined,
        Protocol::FailedDownload => Api::FailedDownload,
        Protocol::Accepted => Api::Accepted,
        Protocol::Downloaded => Api::Downloaded,
        Protocol::InvalidUrl => Api::InvalidUrl,
        Protocol::FailedReload => Api::FailedReload,
        Protocol::Discarded => Api::Discarded,
        Protocol::Unknown(id) => Api::Unknown(id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_pack_results_keep_their_id_and_finality_as_api_status() {
        use infrarust_api::player::ResourcePackStatus;
        use infrarust_protocol::packets::resource_pack::ResourcePackResult;

        for id in 0..8 {
            let result = ResourcePackResult::from_id(id);
            let status = pack_result_to_api(result);
            assert_eq!(status.id(), id);
            assert_eq!(status.is_final(), result.is_final());
        }
        assert_eq!(
            pack_result_to_api(ResourcePackResult::Unknown(42)),
            ResourcePackStatus::Unknown(42)
        );
    }
}
