# infrarust_protocol

Minecraft protocol codec used by the
[Infrarust](https://github.com/Shadowner/Infrarust) reverse proxy.

Implements VarInt/VarLong primitives, packet framing, compression (flate2 or
libdeflate via the `libdeflater` feature), AES/CFB8 encryption, NBT handling,
and the handshake/status/login/play packet types the proxy needs.

MIT licensed (unlike the AGPL proxy) so it can be reused freely in other
Minecraft tooling.

- Repository: <https://github.com/Shadowner/Infrarust>
