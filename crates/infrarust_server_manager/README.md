# infrarust_server_manager

Backend server lifecycle management for the
[Infrarust](https://github.com/Shadowner/Infrarust) Minecraft reverse proxy.

Starts, stops, and monitors backend servers through pluggable providers
(local process, remote API), tracking state transitions so the proxy can wake
servers on demand and put idle ones to sleep.

- Repository: <https://github.com/Shadowner/Infrarust>
