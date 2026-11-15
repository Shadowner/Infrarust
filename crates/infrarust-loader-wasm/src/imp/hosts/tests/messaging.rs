use infrarust_api::permissions::{Capability, CapabilitySet};

use crate::bindings::infrarust::plugin::{messaging, types as wt};

use super::{RecordingPlayer, channel, is_limit_exceeded, quota_state, quotas, state_with};

fn messenger() -> CapabilitySet {
    CapabilitySet::baseline().with(Capability::PluginMessaging)
}

#[tokio::test]
async fn plugin_messages_reach_the_client_and_the_backend_on_registered_channels() {
    let (player, seen) = RecordingPlayer::shared();
    let mut state = state_with(messenger(), vec![player]);

    assert_eq!(
        messaging::Host::register_channel(&mut state, channel())
            .await
            .unwrap(),
        Ok(())
    );
    let channels = messaging::Host::channels(&mut state)
        .await
        .unwrap()
        .unwrap();
    assert!(channels.contains(&channel()), "{channels:?}");

    messaging::Host::send_to_player(&mut state, 1, channel(), b"hi".to_vec())
        .await
        .unwrap()
        .unwrap();
    messaging::Host::send_to_backend(&mut state, 1, channel(), b"up".to_vec())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        *seen.messages.lock().unwrap(),
        [
            ("test:echo".to_owned(), b"hi".to_vec(), false),
            ("test:echo".to_owned(), b"up".to_vec(), true),
        ]
    );

    let invalid = wt::ChannelId {
        modern: Some("Bad Channel".into()),
        legacy: None,
    };
    let refused = messaging::Host::send_to_player(&mut state, 1, invalid, vec![])
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(refused.kind, wt::ErrorKind::InvalidArgument);

    let nobody = messaging::Host::send_to_server(&mut state, "lobby".into(), channel(), vec![])
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(nobody.kind, wt::ErrorKind::Unavailable, "{nobody:?}");

    assert_eq!(
        messaging::Host::unregister_channel(&mut state, channel())
            .await
            .unwrap(),
        Ok(true)
    );
}

#[tokio::test]
async fn the_channel_quota_counts_distinct_channels_and_unregistering_frees_room() {
    let mut state = quota_state(
        quotas(|q| q.plugin_channels = 1),
        &[Capability::PluginMessaging],
    );
    let other = wt::ChannelId {
        modern: Some("test:other".into()),
        legacy: None,
    };
    assert_eq!(
        messaging::Host::register_channel(&mut state, channel())
            .await
            .unwrap(),
        Ok(())
    );
    assert_eq!(
        messaging::Host::register_channel(&mut state, channel())
            .await
            .unwrap(),
        Ok(()),
        "registering a held channel again takes no room"
    );
    let over = messaging::Host::register_channel(&mut state, other.clone())
        .await
        .unwrap();
    assert!(is_limit_exceeded(&over, "plugin_channels"), "{over:?}");

    assert_eq!(
        messaging::Host::unregister_channel(&mut state, channel())
            .await
            .unwrap(),
        Ok(true)
    );
    assert_eq!(
        messaging::Host::register_channel(&mut state, other)
            .await
            .unwrap(),
        Ok(())
    );
}
