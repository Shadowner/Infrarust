//! Shared boilerplate for the raw-`wit_bindgen` sandbox fixtures.
#[allow(clippy::crate_in_macro_def)]
#[macro_export]
macro_rules! raw_fixture {
    (
        $comp:ident,
        id: $id:literal,
        name: $name:literal,
        description: $desc:expr,
        on_enable: $on_enable:block $(,)?
    ) => {
        impl crate::exports::infrarust::plugin::guest::Guest for $comp {
            fn metadata() -> crate::exports::infrarust::plugin::guest::PluginMetadata {
                crate::exports::infrarust::plugin::guest::PluginMetadata {
                    id: ::std::string::String::from($id),
                    name: ::std::string::String::from($name),
                    version: ::std::string::String::from("0.1.0"),
                    authors: ::std::vec::Vec::new(),
                    description: $desc,
                    dependencies: ::std::vec::Vec::new(),
                }
            }
            fn on_enable() -> ::core::result::Result<(), ::std::string::String> $on_enable
            fn on_disable() -> ::core::result::Result<(), ::std::string::String> {
                ::core::result::Result::Ok(())
            }
            fn handle_event(
                _listener: u64,
                _ev: crate::exports::infrarust::plugin::guest::Event,
            ) -> crate::exports::infrarust::plugin::guest::EventOutcome {
                crate::exports::infrarust::plugin::guest::EventOutcome::None
            }
            fn handle_command(
                _callback_id: u64,
                _args: ::std::vec::Vec<::std::string::String>,
                _player: ::core::option::Option<u64>,
            ) {
            }
            fn tab_complete(
                _callback_id: u64,
                _partial: ::std::vec::Vec<::std::string::String>,
                _cursor: u32,
            ) -> ::std::vec::Vec<::std::string::String> {
                ::std::vec::Vec::new()
            }
            fn on_scheduled_task(_callback_id: u64) {}
            fn limbo_on_player_enter(
                _handler: u64,
                _session: &crate::exports::infrarust::plugin::guest::LimboSession,
            ) -> crate::exports::infrarust::plugin::guest::HandlerResult {
                crate::exports::infrarust::plugin::guest::HandlerResult::Accept
            }
            fn limbo_on_command(
                _handler: u64,
                _session: &crate::exports::infrarust::plugin::guest::LimboSession,
                _command: ::std::string::String,
                _args: ::std::vec::Vec<::std::string::String>,
            ) {
            }
            fn limbo_on_chat(
                _handler: u64,
                _session: &crate::exports::infrarust::plugin::guest::LimboSession,
                _message: ::std::string::String,
            ) {
            }
            fn limbo_on_disconnect(_handler: u64, _player: u64) {}
            fn limbo_on_session_end(
                _handler: u64,
                _player: u64,
                _reason: crate::exports::infrarust::plugin::guest::SessionEndReason,
            ) {
            }
            fn permission_level_of(
                _handler: u64,
            ) -> crate::exports::infrarust::plugin::guest::PermissionLevel {
                crate::exports::infrarust::plugin::guest::PermissionLevel::Player
            }
            fn check_permission(_handler: u64, _permission: ::std::string::String) -> bool {
                false
            }
        }

        #[doc(hidden)]
        struct __RawFixtureNoopFilter;

        impl crate::exports::infrarust::plugin::codec_filter::Guest for $comp {
            type FilterInstance = __RawFixtureNoopFilter;
            fn create(
                _factory: u64,
                _init: crate::exports::infrarust::plugin::codec_filter::CodecSessionInit,
            ) -> crate::exports::infrarust::plugin::codec_filter::FilterInstance {
                crate::exports::infrarust::plugin::codec_filter::FilterInstance::new(
                    __RawFixtureNoopFilter,
                )
            }
        }

        impl crate::exports::infrarust::plugin::codec_filter::GuestFilterInstance
            for __RawFixtureNoopFilter
        {
            fn filter(
                &self,
                _packet_id: i32,
                _data: ::std::vec::Vec<u8>,
            ) -> crate::exports::infrarust::plugin::codec_filter::FilterOutput {
                crate::exports::infrarust::plugin::codec_filter::FilterOutput::Pass
            }
            fn on_state_change(
                &self,
                _new_state: crate::exports::infrarust::plugin::codec_filter::ConnectionState,
            ) {
            }
            fn on_compression_change(&self, _threshold: i32) {}
            fn on_encryption_enabled(&self) {}
            fn on_close(&self) {}
        }
    };
}
