use bytes::Bytes;
use infrarust_api::event::ResultedEvent;
use infrarust_api::events::claim::{
    LoginClaimEvent, LoginClaimResult, RawConnectionEvent, RawConnectionResult,
};
use infrarust_api::handoff::{Handoff, HandoffTarget};
use infrarust_api::permissions::Capability;
use infrarust_api::types::ServerId;

use super::{Applied, WasmEvent, unmatched};
use crate::actor::InstanceRef;
use crate::bindings::infrarust::plugin::events::{self as we, EventKind};
use crate::bindings::infrarust::plugin::types as wt;
use crate::convert;

pub(crate) const HANDOFF_NEEDS_CAPABILITY: &str =
    "a handoff outcome needs the connection-handoff capability";

pub(crate) const HANDOFF_ADDRESS_REFUSED: &str =
    "the plugin's network allow-list does not permit the handoff address";

pub(crate) const HANDOFF_NEEDS_GRANTS: &str =
    "a handoff outcome is only accepted from a listening plugin";

pub(crate) const HANDOFF_SERVER_UNKNOWN: &str =
    "the handoff names a server-port target on a server the proxy does not know";

pub(crate) const HANDOFF_PORT_REFUSED: &str = "the plugin's network allow-list does not permit \
     the server-port target on every address of that server";

pub(crate) fn handoff_to_wit(handoff: &Handoff) -> we::Handoff {
    we::Handoff {
        target: match &handoff.target {
            HandoffTarget::Server(server) => we::HandoffTarget::Server(server.as_str().to_owned()),
            HandoffTarget::ServerPort { server, port } => {
                we::HandoffTarget::ServerPort((server.as_str().to_owned(), *port))
            }
            HandoffTarget::Address { host, port } => {
                we::HandoffTarget::Address(wt::ServerAddress {
                    host: host.clone(),
                    port: *port,
                })
            }
            _ => we::HandoffTarget::Route,
        },
        replay: handoff.replay,
        prefix: handoff.prefix.to_vec(),
        reply: handoff.reply.to_vec(),
        proxy_protocol: handoff.proxy_protocol,
    }
}

fn handoff_from_wit(handoff: we::Handoff) -> Handoff {
    let target = match handoff.target {
        we::HandoffTarget::Route => HandoffTarget::Route,
        we::HandoffTarget::Server(server) => HandoffTarget::Server(ServerId::from(server)),
        we::HandoffTarget::ServerPort((server, port)) => HandoffTarget::ServerPort {
            server: ServerId::from(server),
            port,
        },
        we::HandoffTarget::Address(address) => HandoffTarget::Address {
            host: address.host,
            port: address.port,
        },
    };
    let mut native = Handoff::to(target)
        .with_replay(handoff.replay)
        .with_prefix(Bytes::from(handoff.prefix))
        .with_reply(Bytes::from(handoff.reply));
    if let Some(proxy_protocol) = handoff.proxy_protocol {
        native = native.with_proxy_protocol(proxy_protocol);
    }
    native
}

pub(crate) fn admitted_handoff(
    handoff: we::Handoff,
    instance: &InstanceRef,
) -> Result<Handoff, &'static str> {
    if !instance.capabilities().has(Capability::ConnectionHandoff) {
        return Err(HANDOFF_NEEDS_CAPABILITY);
    }
    match &handoff.target {
        we::HandoffTarget::Address(address) => {
            if instance
                .network()
                .permits_handoff(&address.host, address.port)
                .is_err()
            {
                return Err(HANDOFF_ADDRESS_REFUSED);
            }
        }
        we::HandoffTarget::ServerPort((server, port)) => {
            admitted_server_port(server, *port, instance)?;
        }
        we::HandoffTarget::Route | we::HandoffTarget::Server(_) => {}
    }
    Ok(handoff_from_wit(handoff))
}

fn admitted_server_port(
    server: &str,
    port: u16,
    instance: &InstanceRef,
) -> Result<(), &'static str> {
    let config = instance
        .servers()
        .get_server_config(&ServerId::from(server))
        .ok_or(HANDOFF_SERVER_UNKNOWN)?;
    let network = instance.network();
    let permitted = config
        .addresses
        .iter()
        .filter(|address| address.port != port)
        .all(|address| network.permits_handoff(&address.host, port).is_ok());
    if permitted {
        Ok(())
    } else {
        Err(HANDOFF_PORT_REFUSED)
    }
}

impl WasmEvent for RawConnectionEvent {
    const KIND: EventKind = EventKind::RawConnection;

    fn to_wit(&self) -> we::Event {
        we::Event::RawConnection(we::RawConnectionEvent {
            connection_id: self.connection_id,
            remote_addr: convert::socket_to_wit(self.remote_addr),
            local_addr: convert::socket_to_wit(self.local_addr),
            prefix: self.prefix.to_vec(),
            data: self.data.to_vec(),
            result: match self.result() {
                RawConnectionResult::NeedMore(bytes) => we::RawConnectionResult::NeedMore(*bytes),
                RawConnectionResult::Handoff(handoff) => {
                    we::RawConnectionResult::Handoff(handoff_to_wit(handoff))
                }
                RawConnectionResult::Reject => we::RawConnectionResult::Reject,
                _ => we::RawConnectionResult::Pass,
            },
        })
    }

    fn apply(&mut self, outcome: we::EventOutcome) -> Applied {
        let we::EventOutcome::RawConnection(result) = outcome else {
            return unmatched(&outcome);
        };
        self.set_result(match result {
            we::RawConnectionResult::Pass => RawConnectionResult::Pass,
            we::RawConnectionResult::NeedMore(bytes) => RawConnectionResult::NeedMore(bytes),
            we::RawConnectionResult::Reject => RawConnectionResult::Reject,
            we::RawConnectionResult::Handoff(_) => return Applied::Refused(HANDOFF_NEEDS_GRANTS),
        });
        Applied::Set
    }

    fn apply_for(&mut self, outcome: we::EventOutcome, instance: &InstanceRef) -> Applied {
        let we::EventOutcome::RawConnection(we::RawConnectionResult::Handoff(handoff)) = outcome
        else {
            return self.apply(outcome);
        };
        match admitted_handoff(handoff, instance) {
            Ok(handoff) => {
                self.set_result(RawConnectionResult::Handoff(handoff));
                Applied::Set
            }
            Err(reason) => Applied::Refused(reason),
        }
    }
}

impl WasmEvent for LoginClaimEvent {
    const KIND: EventKind = EventKind::LoginClaim;

    fn to_wit(&self) -> we::Event {
        we::Event::LoginClaim(we::LoginClaimEvent {
            remote_addr: convert::socket_to_wit(self.remote_addr),
            virtual_host: self.virtual_host.clone(),
            raw_host: self.raw_host.clone(),
            port: self.port,
            protocol: self.protocol_version.raw(),
            server: convert::server_id_opt(&self.server),
            username: self.username.clone(),
            uuid: self.uuid.map(convert::uuid_to_wit),
            login_start: self.login_start.to_vec(),
            layout_matches: self.layout_matches,
            result: match self.result() {
                LoginClaimResult::Handoff(handoff) => {
                    we::LoginClaimResult::Handoff(handoff_to_wit(handoff))
                }
                _ => we::LoginClaimResult::Pass,
            },
        })
    }

    fn apply(&mut self, outcome: we::EventOutcome) -> Applied {
        match outcome {
            we::EventOutcome::LoginClaim(we::LoginClaimResult::Pass) => {
                self.set_result(LoginClaimResult::Pass);
                Applied::Set
            }
            we::EventOutcome::LoginClaim(we::LoginClaimResult::Handoff(_)) => {
                Applied::Refused(HANDOFF_NEEDS_GRANTS)
            }
            other => unmatched(&other),
        }
    }

    fn apply_for(&mut self, outcome: we::EventOutcome, instance: &InstanceRef) -> Applied {
        let we::EventOutcome::LoginClaim(we::LoginClaimResult::Handoff(handoff)) = outcome else {
            return self.apply(outcome);
        };
        match admitted_handoff(handoff, instance) {
            Ok(handoff) => {
                self.set_result(LoginClaimResult::Handoff(handoff));
                Applied::Set
            }
            Err(reason) => Applied::Refused(reason),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use std::sync::Arc;
    use std::time::Duration;

    use infrarust_api::permissions::CapabilitySet;
    use infrarust_api::services::config_service::{ConfigService, ServerConfig};
    use infrarust_api::test_util::MockConfigService;
    use infrarust_api::types::{ProtocolVersion, ServerAddress};
    use infrarust_config::WasmNetworkConfig;

    use super::*;
    use crate::network::NetworkPolicy;
    use crate::network::resolver::SystemResolver;

    fn raw() -> RawConnectionEvent {
        RawConnectionEvent::new(
            9,
            "203.0.113.7:50000".parse().unwrap(),
            "0.0.0.0:25565".parse().unwrap(),
            &b"XFER"[..],
            &b"XFER\x00"[..],
        )
    }

    fn login() -> LoginClaimEvent {
        LoginClaimEvent::new(
            "203.0.113.7:50000".parse().unwrap(),
            ProtocolVersion::new(767),
            "Steve",
            &b"\x05Steve"[..],
        )
        .with_host("modded.test\0FML3\0", Some("modded.test".into()), 25565)
        .with_server(Some(ServerId::new("modded")))
        .with_uuid(Some(uuid::Uuid::from_u128(7)))
        .with_layout_matches(false)
    }

    fn policy(allow: &[&str]) -> Arc<NetworkPolicy> {
        Arc::new(NetworkPolicy::new(
            "p".to_owned(),
            Some(&WasmNetworkConfig {
                allow: allow.iter().map(|rule| rule.parse().unwrap()).collect(),
                dns: None,
                http: false,
            }),
            true,
            Duration::from_secs(1),
            Arc::new(SystemResolver),
        ))
    }

    fn server(id: &str, addresses: &[(&str, u16)]) -> ServerConfig {
        MockConfigService::server(id).addresses(
            addresses
                .iter()
                .map(|(host, port)| ServerAddress {
                    host: (*host).to_owned(),
                    port: *port,
                })
                .collect(),
        )
    }

    fn servers() -> Arc<dyn ConfigService> {
        Arc::new(
            MockConfigService::new()
                .with_server(server("modded", &[("10.0.0.5", 25565)]))
                .with_server(server("pair", &[("10.0.0.5", 25565), ("10.0.0.6", 25565)]))
                .with_server(server(
                    "mixed",
                    &[("10.0.0.5", 25565), ("MC.example.", 25566)],
                )),
        )
    }

    fn granted_with(network: Arc<NetworkPolicy>) -> InstanceRef {
        InstanceRef::detached_serving(
            CapabilitySet::default().with(Capability::ConnectionHandoff),
            network,
            servers(),
        )
    }

    fn granted(allow: &[&str]) -> InstanceRef {
        granted_with(policy(allow))
    }

    fn wit_handoff(target: we::HandoffTarget) -> we::Handoff {
        we::Handoff {
            target,
            replay: false,
            prefix: b"PRE".to_vec(),
            reply: b"OK".to_vec(),
            proxy_protocol: Some(true),
        }
    }

    #[test]
    fn a_handoff_round_trips_through_the_contract() {
        for handoff in [
            Handoff::route(),
            Handoff::to(HandoffTarget::server("hub")).without_replay(),
            Handoff::to(HandoffTarget::server_port("hub", 30037)).with_prefix(&b"AM"[..]),
            Handoff::to(HandoffTarget::address("files.example", 443))
                .with_reply(&b"hi"[..])
                .with_proxy_protocol(false),
        ] {
            assert_eq!(handoff_from_wit(handoff_to_wit(&handoff)), handoff);
        }
    }

    #[test]
    fn the_guest_sees_the_peeked_bytes_and_the_matched_prefix() {
        let we::Event::RawConnection(record) = raw().to_wit() else {
            panic!("a raw connection is sent as raw-connection");
        };
        assert_eq!(record.connection_id, 9);
        assert_eq!(record.prefix, b"XFER");
        assert_eq!(record.data, b"XFER\x00");
        assert_eq!(record.result, we::RawConnectionResult::Pass);
        assert_eq!(record.remote_addr.port, 50000);
        assert_eq!(record.local_addr.port, 25565);
    }

    #[test]
    fn each_raw_connection_outcome_sets_the_native_one() {
        let instance = granted(&["10.0.0.5:30037"]);
        let mut event = raw();
        event.apply_for(
            we::EventOutcome::RawConnection(we::RawConnectionResult::NeedMore(64)),
            &instance,
        );
        assert_eq!(event.result(), &RawConnectionResult::NeedMore(64));
        event.apply_for(
            we::EventOutcome::RawConnection(we::RawConnectionResult::Reject),
            &instance,
        );
        assert_eq!(event.result(), &RawConnectionResult::Reject);
        let applied = event.apply_for(
            we::EventOutcome::RawConnection(we::RawConnectionResult::Handoff(wit_handoff(
                we::HandoffTarget::ServerPort(("modded".into(), 30037)),
            ))),
            &instance,
        );
        assert_eq!(applied, Applied::Set);
        assert_eq!(
            event.result(),
            &RawConnectionResult::Handoff(
                Handoff::to(HandoffTarget::server_port("modded", 30037))
                    .without_replay()
                    .with_prefix(&b"PRE"[..])
                    .with_reply(&b"OK"[..])
                    .with_proxy_protocol(true)
            )
        );
        let we::Event::RawConnection(record) = event.to_wit() else {
            panic!("a raw connection is sent as raw-connection");
        };
        assert!(matches!(
            record.result,
            we::RawConnectionResult::Handoff(we::Handoff {
                target: we::HandoffTarget::ServerPort(_),
                ..
            })
        ));
        event.apply_for(
            we::EventOutcome::RawConnection(we::RawConnectionResult::Pass),
            &instance,
        );
        assert_eq!(event.result(), &RawConnectionResult::Pass);
    }

    #[test]
    fn a_handoff_without_the_capability_is_ignored() {
        let instance = InstanceRef::detached_with(CapabilitySet::default(), policy(&[]));
        let mut event = raw();
        let applied = event.apply_for(
            we::EventOutcome::RawConnection(we::RawConnectionResult::Handoff(wit_handoff(
                we::HandoffTarget::Route,
            ))),
            &instance,
        );
        assert_eq!(applied, Applied::Refused(HANDOFF_NEEDS_CAPABILITY));
        assert_eq!(event.result(), &RawConnectionResult::Pass);
    }

    #[test]
    fn an_address_handoff_follows_the_network_allow_list() {
        let instance = granted(&["files.example:443", "10.0.0.0/8:30000-31000"]);
        for (host, port, allowed) in [
            ("files.example", 443, true),
            ("FILES.example.", 443, true),
            ("files.example", 80, false),
            ("other.example", 443, false),
            ("10.1.2.3", 30037, true),
            ("10.1.2.3", 25565, false),
            ("192.168.1.2", 30037, false),
        ] {
            let mut event = login();
            let applied = event.apply_for(
                we::EventOutcome::LoginClaim(we::LoginClaimResult::Handoff(wit_handoff(
                    we::HandoffTarget::Address(wt::ServerAddress {
                        host: host.into(),
                        port,
                    }),
                ))),
                &instance,
            );
            if allowed {
                assert_eq!(applied, Applied::Set, "{host}:{port}");
                assert!(matches!(event.result(), LoginClaimResult::Handoff(_)));
            } else {
                assert_eq!(
                    applied,
                    Applied::Refused(HANDOFF_ADDRESS_REFUSED),
                    "{host}:{port}"
                );
                assert_eq!(event.result(), &LoginClaimResult::Pass);
            }
        }

        let no_network = InstanceRef::detached_with(
            CapabilitySet::default().with(Capability::ConnectionHandoff),
            crate::network::probe_policy("p".to_owned()),
        );
        let mut event = login();
        assert_eq!(
            event.apply_for(
                we::EventOutcome::LoginClaim(we::LoginClaimResult::Handoff(wit_handoff(
                    we::HandoffTarget::Address(wt::ServerAddress {
                        host: "files.example".into(),
                        port: 443,
                    }),
                ))),
                &no_network,
            ),
            Applied::Refused(HANDOFF_ADDRESS_REFUSED)
        );
    }

    fn server_port(instance: &InstanceRef, server: &str, port: u16) -> Applied {
        let mut event = raw();
        let applied = event.apply_for(
            we::EventOutcome::RawConnection(we::RawConnectionResult::Handoff(wit_handoff(
                we::HandoffTarget::ServerPort((server.into(), port)),
            ))),
            instance,
        );
        let handed_off = matches!(event.result(), RawConnectionResult::Handoff(_));
        assert_eq!(handed_off, applied == Applied::Set, "{server}:{port}");
        applied
    }

    #[test]
    fn a_server_port_off_the_minecraft_port_follows_the_network_allow_list() {
        let instance = granted(&["10.0.0.5:30037", "mc.example:30000-31000"]);
        for (server, port, expected) in [
            ("modded", 25565, Applied::Set),
            ("modded", 30037, Applied::Set),
            ("modded", 22, Applied::Refused(HANDOFF_PORT_REFUSED)),
            ("modded", 30038, Applied::Refused(HANDOFF_PORT_REFUSED)),
            ("pair", 25565, Applied::Set),
            ("pair", 30037, Applied::Refused(HANDOFF_PORT_REFUSED)),
            ("mixed", 30037, Applied::Set),
            ("mixed", 25565, Applied::Refused(HANDOFF_PORT_REFUSED)),
            ("mixed", 25566, Applied::Refused(HANDOFF_PORT_REFUSED)),
            ("unknown", 25565, Applied::Refused(HANDOFF_SERVER_UNKNOWN)),
            ("unknown", 30037, Applied::Refused(HANDOFF_SERVER_UNKNOWN)),
        ] {
            assert_eq!(
                server_port(&instance, server, port),
                expected,
                "{server}:{port}"
            );
        }

        let no_network = granted_with(crate::network::probe_policy("p".to_owned()));
        assert_eq!(server_port(&no_network, "modded", 25565), Applied::Set);
        assert_eq!(server_port(&no_network, "pair", 25565), Applied::Set);
        assert_eq!(
            server_port(&no_network, "modded", 30037),
            Applied::Refused(HANDOFF_PORT_REFUSED)
        );
        assert_eq!(
            server_port(&no_network, "unknown", 25565),
            Applied::Refused(HANDOFF_SERVER_UNKNOWN)
        );

        let mut event = login();
        assert_eq!(
            event.apply_for(
                we::EventOutcome::LoginClaim(we::LoginClaimResult::Handoff(wit_handoff(
                    we::HandoffTarget::ServerPort(("modded".into(), 3306)),
                ))),
                &instance,
            ),
            Applied::Refused(HANDOFF_PORT_REFUSED)
        );
        assert_eq!(event.result(), &LoginClaimResult::Pass);
    }

    #[test]
    fn a_server_target_is_not_checked_against_the_allow_list() {
        let instance = granted_with(crate::network::probe_policy("p".to_owned()));
        for server in ["modded", "unknown"] {
            let mut event = raw();
            assert_eq!(
                event.apply_for(
                    we::EventOutcome::RawConnection(we::RawConnectionResult::Handoff(wit_handoff(
                        we::HandoffTarget::Server(server.into())
                    ))),
                    &instance,
                ),
                Applied::Set,
                "{server}"
            );
        }
    }

    #[test]
    fn a_login_claim_carries_the_login_start_and_its_layout() {
        let we::Event::LoginClaim(record) = login().to_wit() else {
            panic!("a login claim is sent as login-claim");
        };
        assert_eq!(record.username, "Steve");
        assert_eq!(record.raw_host, "modded.test\0FML3\0");
        assert_eq!(record.virtual_host.as_deref(), Some("modded.test"));
        assert_eq!(record.server.as_deref(), Some("modded"));
        assert_eq!(record.protocol, 767);
        assert_eq!(
            record.uuid,
            Some(convert::uuid_to_wit(uuid::Uuid::from_u128(7)))
        );
        assert_eq!(record.login_start, b"\x05Steve");
        assert!(!record.layout_matches);
        assert_eq!(record.result, we::LoginClaimResult::Pass);

        let mut event = login();
        assert_eq!(
            event.apply_for(
                we::EventOutcome::LoginClaim(we::LoginClaimResult::Handoff(wit_handoff(
                    we::HandoffTarget::Route
                ))),
                &granted(&[]),
            ),
            Applied::Set
        );
        assert_eq!(
            event.apply(we::EventOutcome::LoginClaim(we::LoginClaimResult::Pass)),
            Applied::Set
        );
        assert_eq!(event.result(), &LoginClaimResult::Pass);
        assert_eq!(
            event.apply(we::EventOutcome::LoginClaim(we::LoginClaimResult::Handoff(
                wit_handoff(we::HandoffTarget::Route)
            ))),
            Applied::Refused(HANDOFF_NEEDS_GRANTS)
        );
        assert_eq!(
            event.apply(we::EventOutcome::RawConnection(
                we::RawConnectionResult::Reject
            )),
            Applied::Mismatched
        );
    }
}
