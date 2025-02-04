# Quick Start Monitoring

## Jaeger - Quick Trace Visualization

Run this command to start Jaeger all-in-one container:

```bash
docker run -d \
  --name jaeger \
  -e COLLECTOR_OTLP_ENABLED=true \
  -p 16686:16686 `# UI port` \
  -p 4317:4317 `# OTLP gRPC` \
  -p 4318:4318 `# OTLP HTTP` \
  jaegertracing/all-in-one:latest
```

> one line `docker run -d --name jaeger -e COLLECTOR_OTLP_ENABLED=true -p 16686:16686 -p 4317:4317  -p 4318:4318  jaegertracing/all-in-one:latest`

## Usage

1. Access Jaeger UI: <http://localhost:16686>

2. View traces in real-time through Jaeger UI:
   - Select "infrarust" service
   - Click "Find Traces"
   - See connection flows and timing details

## Common Operations

- View complete connection flow: Look for `TCP Connection` traces
- Monitor backend interactions: Check `wake_up_server` spans
