# Offline Mode

Offline mode enables client/server relay with plugin support, without client authentication.

::: warning
This mode should only be used in controlled environments or when authentication is not required.
:::

## Classic Connection Process

> Assuming there is no encryption request from the server side

```mermaid
sequenceDiagram
    participant C as Client
    participant P as Proxy
    participant S as Server
    
    C->>P: Handshake (Next State: 2)
    C->>P: Login Start
    P->>S: Handshake (Next State: 2)
    P->>S: Login Start
    S->>P: Set Compression
    P->>C: Set Compression
    S->>P: Login Success
    P->>C: Login Success
    C->>P: Login Acknowledged
    P->>S: Login Acknowledged
```

## Configuration

### Minimal Configuration

```yaml
proxy_mode: "offline"
```

### Full Configuration

```yaml
proxy_mode: "offline"
options: ### NOT IMPLEMENTED YET ###
  allow_duplicate_names: false
  max_players: 100
  
plugins: ### NOT IMPLEMENTED YET ###
  enabled: true
  directory: "plugins"
```

## Security

⚠️ This mode offers no client authenticity verification.

### Recommendations

- Use whitelists
- Implement authentication via plugins
- Limit access to local network if possible

## Use Cases

- Local servers
- Test/dev environments
- Private networks
- Cracked servers
