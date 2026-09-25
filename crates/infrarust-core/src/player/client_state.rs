use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use infrarust_api::player::{ChatMode, ClientSettings, MainHand, ParticleStatus, SkinParts};
use infrarust_protocol::packets::play::client_information::ClientInformation;

use crate::plugin_messaging::channels::MAX_KNOWN_CHANNELS;

const NO_PING: u64 = u64::MAX;

#[derive(Default)]
struct Captured {
    brand: Option<String>,
    information: Option<ClientInformation>,
    channels: Vec<String>,
}

pub(crate) struct ClientState {
    captured: Mutex<Captured>,
    ping_micros: AtomicU64,
}

impl Default for ClientState {
    fn default() -> Self {
        Self {
            captured: Mutex::default(),
            ping_micros: AtomicU64::new(NO_PING),
        }
    }
}

impl ClientState {
    fn captured(&self) -> std::sync::MutexGuard<'_, Captured> {
        self.captured.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn brand(&self) -> Option<String> {
        self.captured().brand.clone()
    }

    pub(crate) fn set_brand(&self, brand: String) -> bool {
        let mut captured = self.captured();
        if captured.brand.as_ref() == Some(&brand) {
            return false;
        }
        captured.brand = Some(brand);
        true
    }

    pub(crate) fn information(&self) -> Option<ClientInformation> {
        self.captured().information.clone()
    }

    pub(crate) fn set_information(&self, information: ClientInformation) -> bool {
        let mut captured = self.captured();
        if captured.information.as_ref() == Some(&information) {
            return false;
        }
        captured.information = Some(information);
        true
    }

    pub(crate) fn settings(&self) -> Option<ClientSettings> {
        self.captured().information.as_ref().map(to_settings)
    }

    pub(crate) fn channels(&self) -> Vec<String> {
        self.captured().channels.clone()
    }

    pub(crate) fn add_channels(&self, channels: &[String]) {
        let mut captured = self.captured();
        for channel in channels {
            if captured.channels.len() >= MAX_KNOWN_CHANNELS {
                break;
            }
            if !captured.channels.contains(channel) {
                captured.channels.push(channel.clone());
            }
        }
    }

    pub(crate) fn remove_channels(&self, channels: &[String]) {
        self.captured()
            .channels
            .retain(|known| !channels.contains(known));
    }

    pub(crate) fn record_ping(&self, rtt: Duration) {
        let micros = u64::try_from(rtt.as_micros()).unwrap_or(NO_PING - 1);
        self.ping_micros
            .store(micros.min(NO_PING - 1), Ordering::Relaxed);
    }

    pub(crate) fn ping(&self) -> Option<Duration> {
        match self.ping_micros.load(Ordering::Relaxed) {
            NO_PING => None,
            micros => Some(Duration::from_micros(micros)),
        }
    }
}

pub(crate) fn to_settings(information: &ClientInformation) -> ClientSettings {
    let mut settings = ClientSettings::new(information.locale.clone());
    settings.view_distance = u8::try_from(information.view_distance).unwrap_or(0);
    settings.chat_mode = ChatMode::from_id(information.chat_mode);
    settings.chat_colors = information.chat_colors;
    settings.skin_parts = SkinParts::new(information.displayed_skin_parts);
    settings.main_hand = MainHand::from_id(information.main_hand);
    settings.text_filtering = information.text_filtering;
    settings.allow_listing = information.allow_server_listings;
    settings.particle_status = ParticleStatus::from_id(information.particle_status);
    settings
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn information() -> ClientInformation {
        ClientInformation {
            locale: "de_de".to_string(),
            view_distance: 6,
            chat_mode: 2,
            chat_colors: false,
            difficulty: 0,
            displayed_skin_parts: 0x41,
            main_hand: 0,
            text_filtering: true,
            allow_server_listings: false,
            particle_status: 1,
        }
    }

    #[test]
    fn settings_map_every_field() {
        let settings = to_settings(&information());
        assert_eq!(settings.locale, "de_de");
        assert_eq!(settings.view_distance, 6);
        assert_eq!(settings.chat_mode, ChatMode::Hidden);
        assert!(!settings.chat_colors);
        assert!(settings.skin_parts.cape() && settings.skin_parts.hat());
        assert!(!settings.skin_parts.jacket());
        assert_eq!(settings.main_hand, MainHand::Left);
        assert!(settings.text_filtering);
        assert!(!settings.allow_listing);
        assert_eq!(settings.particle_status, ParticleStatus::Decreased);
    }

    #[test]
    fn changes_are_reported_once() {
        let state = ClientState::default();
        assert!(state.set_information(information()));
        assert!(!state.set_information(information()));
        assert!(state.set_brand("vanilla".into()));
        assert!(!state.set_brand("vanilla".into()));
        assert!(state.set_brand("fabric".into()));
    }

    #[test]
    fn channels_are_a_capped_ordered_set() {
        let state = ClientState::default();
        state.add_channels(&["a:a".into(), "b:b".into(), "a:a".into()]);
        state.remove_channels(&["a:a".into()]);
        state.add_channels(&["c:c".into()]);
        assert_eq!(state.channels(), vec!["b:b", "c:c"]);
        let many: Vec<String> = (0..MAX_KNOWN_CHANNELS * 2)
            .map(|i| format!("m:{i}"))
            .collect();
        state.add_channels(&many);
        assert_eq!(state.channels().len(), MAX_KNOWN_CHANNELS);
    }

    #[test]
    fn ping_is_unknown_until_measured() {
        let state = ClientState::default();
        assert_eq!(state.ping(), None);
        state.record_ping(Duration::from_millis(42));
        assert_eq!(state.ping(), Some(Duration::from_millis(42)));
    }
}
