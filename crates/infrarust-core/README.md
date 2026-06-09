# infrarust-core

Core runtime of the [Infrarust](https://github.com/Shadowner/Infrarust)
Minecraft reverse proxy.

Implements connection handling (passthrough and intercepted modes), domain
routing, load balancing, the status/MOTD relay, the ban system, server
lifecycle orchestration, the limbo engine, and the native plugin host. The
`docker` and `telemetry` features gate the Docker provider and OpenTelemetry
export respectively.

Used by the `infrarust` binary; not intended as a standalone library API
surface while 2.0 is in beta.

- Repository: <https://github.com/Shadowner/Infrarust>
- Documentation: <https://infrarust.dev>
