# infrarust_protocol

Minecraft protocol codec used by the
[Infrarust](https://github.com/Shadowner/Infrarust) reverse proxy.

Implements VarInt/VarLong primitives, packet framing, compression (flate2 or
libdeflate via the `libdeflater` feature), AES/CFB8 encryption, NBT skipping,
and the handshake/status/login/config/play packet types the proxy needs.

Each packet type carries its own per-protocol-version packet-ID table through
the `Packet` trait, so adding a version means editing the packet, not a central
table. The registry indexes those tables for O(1) decode and encode lookup.

MIT licensed (unlike the AGPL proxy) so it can be reused freely in other
Minecraft tooling.

- Repository: <https://github.com/Shadowner/Infrarust>
