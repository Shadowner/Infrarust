# Telemetry and Monitoring

Infrarust provides comprehensive telemetry through OpenTelemetry, including metrics, traces, and logs.

## Configuration

Enable telemetry in your `config.yaml`:

```yaml
telemetry:
  enabled: true                    # Enable telemetry collection
  export_interval_seconds: 30      # Export interval
  export_url: "http://127.0.0.1:4317"  # OTLP endpoint
  enable_metrics: true            # Enable metrics collection
  enable_tracing: true           # Enable distributed tracing
```

## Available Metrics

### Connection Metrics

- `connections.active` - Current number of active connections
- `connections.errors` - Number of connection errors
- `network.bytes` - Current bytes transferred
- `network.bytes.total` - Total bytes transferred since start
- `connections.latency` - Connection latency
- `requests.rate` - Number of requests per second

### Backend Metrics

- `backends.active` - Number of active backend servers
- `backends.latency` - Backend server response time
- `backends.errors` - Number of backend errors
- `backends.requests` - Total backend requests

### System Metrics

- `system.cpu` - CPU usage percentage
- `system.memory` - Memory usage
- `system.open_files` - Number of open files
- `system.threads` - Number of threads
- `system.internal_errors` - Number of internal errors

### Minecraft-Specific Metrics

- `minecraft.protocol_errors` - Number of Minecraft protocol errors
- `minecraft.players` - Number of connected players
- `minecraft.packet_time` - Packet processing time

## Quick Start Monitoring Stack

Infrarust includes a ready-to-use monitoring stack in the `docker/monitoring` directory.

### Prerequisites

- Docker
- Docker Compose

### Start the Monitoring Stack

```bash
cd docker/monitoring
docker compose up -d
```

This will start:

- Grafana (UI: http://127.0.0.1:3000)
- Prometheus (UI: http://127.0.0.1:9090)
- Tempo (Traces)
- OpenTelemetry Collector

### Configuration Files

#### OpenTelemetry Collector

```yaml
receivers:
  otlp:
    protocols:
      grpc:
        endpoint: "0.0.0.0:4317"
      http:
        endpoint: "0.0.0.0:4318"

processors:
  batch:

exporters:
  prometheus:
    endpoint: "0.0.0.0:8889"
  otlp:
    endpoint: "tempo:4317"

service:
  pipelines:
    metrics:
      receivers: [otlp]
      processors: [batch]
      exporters: [prometheus]
    traces:
      receivers: [otlp]
      processors: [batch]
      exporters: [otlp]
```

### Accessing the Monitoring Stack

1. **Grafana**: http://127.0.0.1:3000
   - Default credentials: admin/admin
   - Preconfigured dashboards available

2. **Prometheus**: http://127.0.0.1:9090
   - Direct access to metrics
   - Query interface for metric exploration

3. **Tempo**: Accessed through Grafana
   - Distributed tracing visualization
   - Trace search and analysis

### Available Dashboards

The monitoring stack includes pre-configured dashboards for:

- Global Dashboard

### Trace Examples

Common traces available:

- TCP Connection Flow
- Packet Processing
- Configuration Provider Setup
- Configuration Update

### Metrics Examples

```promql
# Active Connections
rate(connections_active_total[5m])

# Backend Latency
histogram_quantile(0.95, sum(rate(backends_latency_bucket[5m])) by (le))

# Protocol Errors
sum(minecraft_protocol_errors_total) by (error_type)
```

## Troubleshooting

### Common Issues

1. **No metrics appearing**
   - Verify telemetry configuration is enabled
   - Check OTLP endpoint accessibility
   - Verify collector is running

2. **High latency in collection**
   - Adjust batch processing settings
   - Check network connectivity
   - Review export interval settings

### Debug Mode

Enable debug logging for more detailed telemetry information:

```yaml
logging:
  level: debug  # Not implemented yet
```

## Additional Resources

- [OpenTelemetry Documentation](https://opentelemetry.io/docs/)
- [Grafana Documentation](https://grafana.com/docs/)
- [Prometheus Documentation](https://prometheus.io/docs/)
