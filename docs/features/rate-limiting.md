# Rate Limiting

Rate limiting is an essential security mechanism that controls the number of connections per IP address within a given time interval.

## Configuration

To enable rate limiting, add the following configuration to your `config.yaml` file:

```yaml
security:
  rateLimiter:
    # Maximum number of requests allowed per time window
    requestLimit: 10
    
    # Duration of the time window (in seconds)
    windowDuration: 1
    
    # Block duration after limit exceeded (in seconds)
    blockDuration: 300
```

## Parameters

| Parameter | Description | Default Value |
|-----------|-------------|---------------|
| `requestLimit` | Maximum number of requests | 10 |
| `windowDuration` | Window duration (seconds) | 1 |
| `blockDuration` | Block duration (seconds) | 300 |

## Operation

1. A sliding window is maintained for each IP address
2. Each new connection increments a counter
3. If the counter exceeds `requestLimit` within the window:
   - The IP is blocked for `blockDuration`
   - New connections are rejected
   - An error message is sent to the client

## DDoS Protection

Rate limiting is part of the DDoS protection strategy along with:

- IP filtering `### NOT IMPLEMENTED ###`
- Subnet limitations `### NOT IMPLEMENTED ###`
- Adaptive thresholds `### NOT IMPLEMENTED ###`
- Temporary blacklist `### NOT IMPLEMENTED ###`

## Configuration Examples

### Basic Configuration

```yaml
security:
  rateLimiter:
    requestLimit: 10
    windowDuration: 1
```

### Strict Configuration

```yaml
security:
  rateLimiter:
    requestLimit: 5
    windowDuration: 1
    blockDuration: 600
```

## Monitoring - Not implemented

The rate limiter will expose metrics in the future:

- Number of blocked connections
- Currently blocked IPs
- Block rate
- Connection spikes

See [Monitoring](../deployment/monitoring.md) for more details.

