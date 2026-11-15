use std::sync::Arc;

use infrarust_api::permissions::CapabilitySet;
use infrarust_api::types::Component;

use crate::bindings::infrarust::plugin::events;

use super::{state_with, text_of};

#[tokio::test]
async fn the_ping_fields_are_read_only_while_a_ping_is_handled() {
    let mut state = state_with(CapabilitySet::baseline(), vec![]);
    assert_eq!(
        events::Host::ping_description(&mut state).await.unwrap(),
        None
    );
    assert_eq!(events::Host::ping_favicon(&mut state).await.unwrap(), None);
    assert!(
        events::Host::ping_player_sample(&mut state)
            .await
            .unwrap()
            .is_empty()
    );

    state.set_event_details(Some(Arc::new(crate::events::EventDetails::Ping(
        crate::events::PingDetails {
            description: Component::text("motd"),
            favicon: Some("data:image/png;base64,AAAA".to_owned()),
            player_sample: vec![("Notch".to_owned(), uuid::Uuid::from_u128(7))],
        },
    ))));
    assert_eq!(
        events::Host::ping_description(&mut state).await.unwrap(),
        Some(text_of("motd"))
    );
    assert_eq!(
        events::Host::ping_favicon(&mut state)
            .await
            .unwrap()
            .as_deref(),
        Some("data:image/png;base64,AAAA")
    );
    let sample = events::Host::ping_player_sample(&mut state).await.unwrap();
    assert_eq!(sample.len(), 1);
    assert_eq!(sample[0].name, "Notch");

    state.begin_call(None);
    assert_eq!(
        events::Host::ping_description(&mut state).await.unwrap(),
        None
    );
}
