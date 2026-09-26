use infrarust_protocol::packets::Packet;
use infrarust_protocol::registry::{PacketRegistry, build_default_registry};
use infrarust_protocol::version::ProtocolVersion;

pub fn assert_ids<P: Packet>(expected: &[(i32, Option<i32>)]) {
    let registry = build_default_registry();
    let mismatches: Vec<String> = expected
        .iter()
        .filter_map(|&(protocol, id)| {
            let actual = registry.get_packet_id::<P>(ProtocolVersion(protocol));
            (actual != id)
                .then(|| format!("protocol {protocol}: expected {id:02X?}, got {actual:02X?}"))
        })
        .collect();
    assert!(
        mismatches.is_empty(),
        "{} id mismatches:\n{}",
        P::NAME,
        mismatches.join("\n")
    );
}

pub fn assert_decodable<P: Packet>(registry: &PacketRegistry, expected: &[(i32, Option<i32>)]) {
    for &(protocol, id) in expected {
        if let Some(id) = id {
            assert!(
                registry.has_decoder(P::STATE, P::DIRECTION, ProtocolVersion(protocol), id),
                "{} has no decoder on 0x{id:02X} at protocol {protocol}",
                P::NAME
            );
        }
    }
}
