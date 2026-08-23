# infrarust-transport

Low-level networking layer of the
[Infrarust](https://github.com/Shadowner/Infrarust) Minecraft reverse proxy.

Provides the TCP listener, backend connection establishment, bidirectional
forwarding (including zero-copy splice on Linux), and HAProxy PROXY protocol
v1/v2 support for preserving client addresses.

- Repository: <https://github.com/Shadowner/Infrarust>
