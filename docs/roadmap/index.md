# Infrarust Roadmap

<style>
.feature-list {
  padding-left: 1.5rem;
  margin-bottom: 2rem;
}

.phase-badge {
  display: inline-block;
  padding: 0.25rem 0.75rem;
  border-radius: 1rem;
  font-size: 0.875rem;
  font-weight: 500;
  margin-right: 0.5rem;
}

.phase-badge.current {
  background: var(--vp-c-brand-1);
  color: var(--vp-c-soft-3);
}

.phase-badge.next {
  background: var(--vp-c-default-1);
}

.completed {
  color: var(--vp-c-green);
}
</style>

::: info Current Status
Infrarust is under active development. This roadmap outlines our planned features and enhancements.
:::

# Development Phases

## <span class="phase-badge current">Current</span> Phase 0: Core Foundation

<div class="feature-list completed">

- ✅ Basic reverse proxy with hostname routing
- ✅ OpenTelemetry integration
- ✅ Grafana dashboard templates
- ✅ Protocol-agnostic passthrough mode
- ✅ Hot-reload configuration
- ✅ Modular configuration provider

</div>

## <span class="phase-badge next">Next</span> Phase 1: Architecture Refactoring

<div class="feature-list">

### Valence Integration

- 🔄 Protocol handling adaptation
- 🔄 Packet management system
- 🔄 Connection state machine

### Multi-Crate Architecture

- 📦 Feature modularization
- 📦 Platform-specific optimizations
- 📦 Clean architecture patterns

</div>

## Phase 2: Data & Configuration

<div class="feature-list">

### Storage Layer

- 💾 Async SQLx (PostgreSQL/SQLite)
- 💾 Schema versioning (Refinery)
- 💾 Redis caching integration

### Enhanced Configuration

- 🔐 Secret management
- 🔐 Encrypted credentials
- 🔐 Secure key storage

</div>

## Phase 3: Command Framework

<div class="feature-list">

### Access Control

- 👥 RBAC implementation
- 👥 Permission hierarchy
- 👥 Minecraft-style config

### Management Features

- 🎮 REPL interface
- 🎮 State management
- 🎮 Webhook system
- 🎮 Audit logging

</div>

## Phase 4: Advanced Load Balancing

<div class="feature-list">

### Load Distribution

- ⚖️ Weighted round-robin
- ⚖️ Connection-based balancing
- ⚖️ Latency-aware routing
- ⚖️ Session persistence

### High Availability

- 🔄 Server pooling
- 🔄 Automatic failover
- 🔄 Graceful shutdown
- 🔄 Multi-proxy clustering

</div>

## Phase 5: Administration Interface

<div class="feature-list">

### Backend API

- 🔌 RESTful endpoints
- 🔌 Real-time updates
- 🔌 Metrics aggregation

### Monitoring Suite

- 📊 Traffic visualization
- 📊 Alert configuration
- 📊 Performance analytics

### Control Panel

- 🎛️ JWT authentication
- 🎛️ Configuration UI
- 🎛️ Connection monitoring

</div>

## Phase 6: Plugin Architecture

<div class="feature-list">

### Core System

- 🧩 WASM runtime
- 🧩 Plugin derive macros
- 🧩 Dynamic loading

### Integration

- 🔌 Packet interception
- 🔌 Platform compatibility
- 🔌 Event system

</div>

## Phase 7: Network Optimizations

<div class="feature-list">

### Protocol Support

- 🌐 Version translation (1.8→1.20+)
- 🌐 QUIC implementation
- 🌐 Zero-copy optimization

### Security Features

- 🛡️ BungeeGuard integration
- 🛡️ Advanced rate limiting
- 🛡️ DDoS protection

</div>

::: warning Future Considerations
The following features are being evaluated but are not currently on the roadmap:

## Authentication System

- 🔒 Custom session management
- 🔒 Offline mode support
- 🔒 Third-party integration
:::
