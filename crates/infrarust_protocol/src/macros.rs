macro_rules! ids {
    ( $( $from:ident $( ..= $to:ident )? => $id:literal ),* $(,)? ) => {
        &[ $(
            $crate::packets::PacketMapping {
                id: $id,
                from: $crate::version::ProtocolVersion::$from,
                to: ids!(@end $($to)?),
            }
        ),* ]
    };
    (@end) => { None };
    (@end $to:ident) => { Some($crate::version::ProtocolVersion::$to) };
}

macro_rules! define_twin_packets {
    (
        packets: {
            $(
                $( #[$pmeta:meta] )*
                $name:ident : $direction:ident = $ids:expr
            ),+ $(,)?
        },
        state: $state:expr,
        encode_only: $encode_only:expr,
        fields: $fields:tt,
        shared_impl: $shared_impl:tt,
        decode($r:ident, $decode_ver:ident): $decode_body:expr,
        encode($self_:ident, $w:ident, $encode_ver:ident): $encode_body:expr $(,)?
    ) => {
        $(
            define_twin_packets!(@one
                $( #[$pmeta] )*
                $name, $state, $crate::version::Direction::$direction, $ids, $encode_only,
                $fields,
                decode($r, $decode_ver): $decode_body,
                encode($self_, $w, $encode_ver): $encode_body
            );

            impl $name $shared_impl
        )+
    };
    (
        clientbound: $c_name:ident,
        serverbound: $s_name:ident,
        state: $state:expr,
        clientbound_ids: $c_ids:expr,
        serverbound_ids: $s_ids:expr,
        encode_only: $encode_only:expr,
        fields: { $( pub $field:ident : $ty:ty ),* $(,)? },
        decode($r:ident, $decode_ver:ident): $decode_body:expr,
        encode($self_:ident, $w:ident, $encode_ver:ident): $encode_body:expr $(,)?
    ) => {
        define_twin_packets!(@one
            $c_name, $state, $crate::version::Direction::Clientbound, $c_ids, $encode_only,
            { $( pub $field : $ty ),* },
            decode($r, $decode_ver): $decode_body,
            encode($self_, $w, $encode_ver): $encode_body
        );
        define_twin_packets!(@one
            $s_name, $state, $crate::version::Direction::Serverbound, $s_ids, $encode_only,
            { $( pub $field : $ty ),* },
            decode($r, $decode_ver): $decode_body,
            encode($self_, $w, $encode_ver): $encode_body
        );
    };
    (@one
        $( #[$pmeta:meta] )*
        $name:ident, $state:expr, $direction:expr, $ids:expr, $encode_only:expr,
        { $( pub $field:ident : $ty:ty ),* $(,)? },
        decode($r:ident, $decode_ver:ident): $decode_body:expr,
        encode($self_:ident, $w:ident, $encode_ver:ident): $encode_body:expr
    ) => {
        $( #[$pmeta] )*
        #[derive(Debug, Clone)]
        pub struct $name {
            $( pub $field : $ty, )*
        }

        impl $crate::packets::Packet for $name {
            const NAME: &'static str = stringify!($name);
            const STATE: $crate::version::ConnectionState = $state;
            const DIRECTION: $crate::version::Direction = $direction;
            const IDS: &'static [$crate::packets::PacketMapping] = $ids;
            const ENCODE_ONLY: bool = $encode_only;

            fn decode($r: &mut &[u8], $decode_ver: $crate::version::ProtocolVersion)
                -> $crate::error::ProtocolResult<Self>
            {
                $decode_body
            }

            #[allow(unused_mut)]
            fn encode(
                &$self_,
                mut $w: &mut (impl std::io::Write + ?Sized),
                $encode_ver: $crate::version::ProtocolVersion,
            ) -> $crate::error::ProtocolResult<()> {
                $encode_body
            }
        }
    };
}
