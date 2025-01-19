# Full Mode (Non-Functional)

⚠️ **This mode is currently non-functional and cannot be used.**

## Initial Objective

This mode aimed to combine:

- Plugin support
- Complete authentication
- Servers in `online_mode=true`

## Technical Limitation

```mermaid
sequenceDiagram
    participant C as Client
    participant P as Proxy
    participant S as Server
    
    C->>P: Handshake (0x00)
    C->>P: Login Start (0x00)
    P->>S: Forward packets

    S->>P: Encryption Request (0x01)
    P->>C: Forward Packet (0x01)
    Note over C,S: ❌ Unable to decrypt <br/>client shared secret

    C->>P: Encryption Response (0x01)
    P->>P: Decrypt shared secret
    P->>S: Forward Encryption Response (0x01)
```

## Reason for Failure

Full mode cannot work because:

1. The server and client use an external API
2. The process depends on an encrypted shared secret
3. The proxy cannot decrypt and relay this secret
