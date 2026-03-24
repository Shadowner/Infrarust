
# Cache

Infrarust's cache system optimizes performance by temporarily storing server responses.

## Configuration

The caching system can be configured globally in `config.yaml` and overridden per server in proxy configurations:

```yaml
# Global cache configuration in config.yaml
cache:
  status_ttl_seconds: 30       # TTL for status cache entries (seconds)
  max_status_entries: 1000     # Maximum number of status cache entries

# Server-specific cache in proxies/*.yml
caches:
  status_ttl_seconds: 15       # Override TTL for this server
  max_status_entries: 500      # Override max entries for this server
```

### Default Values

If not specified, these defaults are used:
- `status_ttl_seconds`: 30 seconds
- `max_status_entries`: 1000 entries

## Cache Types

### Status Cache

The Status Cache stores server ping/status responses:

- Reduces load on backend servers by caching status responses
- Provides faster response times for client server list requests
- Automatically invalidates entries after TTL expires
- Uses domain and protocol version as cache keys

### Benefits of Status Caching

- **Protection**: Shields backend servers from status request floods
- **Performance**: Reduces response time for the server list screen
- **Consistency**: Provides stable responses even when backend servers are busy

## Caching Behavior

When a client requests a server's status:

1. Infrarust checks if a valid cached status exists
2. If found and not expired, returns the cached status
3. If not found or expired, forwards the request to the backend server
4. Caches the new response for future requests

## Advanced Features

### Cache Invalidation

Cached status entries are invalidated:
- Automatically after TTL expiration
- When the associated server configuration changes
- When the server becomes unreachable

### Memory Management

Infrarust limits memory usage with the `max_status_entries` setting, which caps the number of cached status responses. When this limit is reached, the oldest entries are evicted first.

## Future Enhancements

The following features are planned but not yet implemented:

- **Memory Limits**: Precise memory usage control
```yaml
cache:
  memory_limit_mb: 512        # Maximum memory usage
  cleanup_interval: 60        # Cleanup interval in seconds
```

- **Cache Compression**: Reduce memory footprint through compression
- **Smart Eviction**: More sophisticated cache eviction strategies
- **Cache Persistence**: Optional disk persistence for cache data
- **Cache Metrics**: Detailed statistics on cache performance

## Cache Metrics (Planned)

In future releases, the cache will expose detailed metrics:

- Hit/miss ratio
- Memory usage
- Average response time
- Number of active entries
- Eviction rate

These metrics will be available through Infrarust's telemetry system.
