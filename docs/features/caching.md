# Cache

Infrarust's cache system optimizes performance by temporarily storing server responses.

## Configuration - Not implemented

```yaml
cache:
  # Cache entry lifetime (in seconds)
  ttl: 30
  
  # Maximum cache size in memory (in MB)
  maxSize: 100
  
  # Enable status response caching
  statusCache: true
```

## Cache Types

### Status Cache

- Stores server ping/status responses
- Key format: `domain:version`
- Reduces backend server load
- Automatic update on expiration

## Optimizations

### Memory Management - Not implemented

```yaml
cache:
  memoryLimit: 512 # MB
  cleanupInterval: 60 # seconds
```

### Performance

- Using hashmaps for O(1) access
- Asynchronous cleanup
- In-memory data compression
- Smart eviction (LRU)

## Metrics - Not implemented

The cache exposes statistics:

- Hit/miss rate
- Memory usage
- Average response time
- Active entries
