use super::{GuestEvent, ResultCell};
use crate::bindings::event_bus as wb;
use crate::bindings::events::{self as we, Event, EventKind, EventOutcome};
use crate::bindings::types as wt;
use crate::codec::ConnectionState;
use crate::types::{PacketDirection, PlayerId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacketFilter {
    pub packet_id: i32,
    pub state: ConnectionState,
    pub direction: PacketDirection,
}

impl PacketFilter {
    #[must_use]
    pub const fn new(packet_id: i32, state: ConnectionState, direction: PacketDirection) -> Self {
        Self {
            packet_id,
            state,
            direction,
        }
    }

    #[must_use]
    pub const fn serverbound(packet_id: i32, state: ConnectionState) -> Self {
        Self::new(packet_id, state, PacketDirection::Serverbound)
    }

    #[must_use]
    pub const fn clientbound(packet_id: i32, state: ConnectionState) -> Self {
        Self::new(packet_id, state, PacketDirection::Clientbound)
    }

    pub(crate) const fn to_wit(self) -> wb::PacketFilter {
        wb::PacketFilter {
            packet_id: self.packet_id,
            state: self.state,
            direction: self.direction.to_wit(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RawPacketResult {
    Pass,
    Modify { packet_id: i32, data: Vec<u8> },
    Drop,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RawPacketEvent {
    pub player: PlayerId,
    pub direction: PacketDirection,
    pub packet_id: i32,
    pub data: Vec<u8>,
    result: ResultCell<RawPacketResult>,
}

impl RawPacketEvent {
    #[must_use]
    pub const fn result(&self) -> &RawPacketResult {
        self.result.get()
    }

    pub fn set_result(&mut self, result: RawPacketResult) {
        self.result.set(result);
    }

    pub fn pass(&mut self) {
        self.set_result(RawPacketResult::Pass);
    }

    pub fn drop_packet(&mut self) {
        self.set_result(RawPacketResult::Drop);
    }

    pub fn modify(&mut self, packet_id: i32, data: impl Into<Vec<u8>>) {
        self.set_result(RawPacketResult::Modify {
            packet_id,
            data: data.into(),
        });
    }
}

impl GuestEvent for RawPacketEvent {
    const KIND: EventKind = EventKind::RawPacket;

    fn from_event(ev: Event) -> Option<Self> {
        let Event::RawPacket(e) = ev else {
            return None;
        };
        Some(Self {
            player: PlayerId::new(e.player),
            direction: PacketDirection::from_wit(e.direction),
            packet_id: e.packet.packet_id,
            data: e.packet.data,
            result: ResultCell::new(match e.result {
                we::RawPacketResult::Pass => RawPacketResult::Pass,
                we::RawPacketResult::Modify(packet) => RawPacketResult::Modify {
                    packet_id: packet.packet_id,
                    data: packet.data,
                },
                we::RawPacketResult::Drop => RawPacketResult::Drop,
            }),
        })
    }

    fn into_outcome(self) -> EventOutcome {
        self.result
            .into_changed()
            .map_or(EventOutcome::Unchanged, |r| {
                EventOutcome::RawPacket(match r {
                    RawPacketResult::Pass => we::RawPacketResult::Pass,
                    RawPacketResult::Modify { packet_id, data } => {
                        we::RawPacketResult::Modify(wt::RawPacket { packet_id, data })
                    }
                    RawPacketResult::Drop => we::RawPacketResult::Drop,
                })
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dropped_packet_answers_drop() {
        let mut event = RawPacketEvent::from_event(Event::RawPacket(we::RawPacketEvent {
            player: 4,
            direction: wt::PacketDirection::Serverbound,
            packet: wt::RawPacket {
                packet_id: 5,
                data: vec![1, 2],
            },
            result: we::RawPacketResult::Pass,
        }))
        .unwrap();
        assert_eq!(event.player, PlayerId::new(4));
        assert_eq!(event.data, [1, 2]);
        event.drop_packet();
        assert_eq!(
            event.into_outcome(),
            EventOutcome::RawPacket(we::RawPacketResult::Drop)
        );
    }
}
