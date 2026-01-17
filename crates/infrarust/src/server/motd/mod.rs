mod favicon;
mod generator;
mod response;
mod state;

pub use favicon::{INFRARUST_FAVICON, parse_favicon};
pub use generator::{generate_for_state, generate_motd_packet, get_motd_config_for_state};
pub use response::{
    generate_response, handle_server_fetch_error, handle_server_fetch_error_with_shared,
};
pub use state::MotdState;

#[allow(deprecated)]
pub use response::{
    generate_crashing_motd_response, generate_imminent_shutdown_motd_response,
    generate_not_started_motd_response, generate_online_motd_response,
    generate_starting_motd_response, generate_stopping_motd_response,
    generate_unable_status_motd_response, generate_unknown_server_response,
    generate_unknown_status_server_response, generate_unreachable_motd_response,
};

#[deprecated(since = "0.2.0", note = "Use INFRARUST_FAVICON instead")]
pub use favicon::INFRARUST_FAVICON as FAVICON;
