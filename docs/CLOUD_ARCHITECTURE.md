# Cloud Platform Architecture

This document describes the architecture of Craton's cloud platform, which provides managed database services on top of the core Craton engine.

## Overview

The cloud platform consists of two layers:
1. **Core Layer** (`crates/craton-*`) - The Craton database engine
2. **Platform Layer** (`platform/crates/*`) - Multi-tenant control plane

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                         Craton Cloud Platform                              │
│                                                                              │
│  ┌────────────────────────────────────────────────────────────────────────┐ │
│  │                         Platform Layer                                  │ │
│  │                                                                         │ │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌───────────────┐  │ │
│  │  │ platform-   │  │ platform-   │  │ platform-   │  │ platform-     │  │ │
│  │  │ app (HTTP)  │  │ identity    │  │ fleet       │  │ data          │  │ │
│  │  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘  └───────┬───────┘  │ │
│  │         │                │                │                  │          │ │
│  │  ┌──────┴────────────────┴────────────────┴──────────────────┴───────┐  │ │
│  │  │                    platform-nats (Event Sourcing)                  │  │ │
│  │  └────────────────────────────────────────────────────────────────────┘  │ │
│  │         │                                                     │          │ │
│  │  ┌──────┴───────────────┐          ┌─────────────────────────┴───────┐  │ │
│  │  │ platform-sqlite      │          │ platform-kernel (Shared Types)  │  │ │
│  │  │ (Read Projections)   │          │ OrgId, ClusterId, UserId        │  │ │
│  │  └──────────────────────┘          └─────────────────────────────────┘  │ │
│  └────────────────────────────────────────────────────────────────────────┘ │
│                                      │                                       │
│                                      ▼                                       │
│  ┌────────────────────────────────────────────────────────────────────────┐ │
│  │                          Core Layer (craton-*)                             │ │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌───────────────┐  │ │
│  │  │ craton-server  │  │ craton-vsr     │  │ craton-store   │  │ craton-query     │  │ │
│  │  │ (RPC)       │  │ (Consensus) │  │ (B+tree)    │  │ (SQL)         │  │ │
│  │  └─────────────┘  └─────────────┘  └─────────────┘  └───────────────┘  │ │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌───────────────┐  │ │
│  │  │ craton-kernel  │  │ craton-storage │  │ craton-crypto  │  │ craton-types     │  │ │
│  │  │ (State)     │  │ (Log)       │  │ (Hash/Sign) │  │ (IDs)         │  │ │
│  │  └─────────────┘  └─────────────┘  └─────────────┘  └───────────────┘  │ │
│  └────────────────────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## Platform Crate Structure

### platform-kernel

Shared primitives and type definitions used across all platform crates.

```rust
// UUIDv7 identifiers for all entities
pub struct OrgId(Uuid);
pub struct UserId(Uuid);
pub struct ClusterId(Uuid);
pub struct NodeId(Uuid);
pub struct SessionId(Uuid);
```

**Key Features**:
- UUIDv7 for time-ordered, globally unique identifiers
- Type-safe ID wrappers (cannot confuse `OrgId` with `UserId`)
- Shared error types and result aliases

### platform-identity

Authentication, users, organizations, and RBAC.

```
platform-identity/
├── domain/           # Pure functional core (decision.rs)
│   ├── user.rs       # User aggregates
│   ├── org.rs        # Organization aggregates
│   └── session.rs    # Session management
├── auth/             # Authentication mechanisms
│   ├── webauthn.rs   # Passkey/WebAuthn support
│   ├── oauth.rs      # OAuth providers (GitHub, Google, Microsoft)
│   ├── middleware.rs # Auth middleware
│   └── rate_limit.rs # Auth-specific rate limiting
├── workflows/        # Command orchestration
│   └── signup.rs     # User signup workflow
├── projections/      # SQLite read models
│   └── user.rs       # User projections
└── http/             # API routes
    ├── session.rs    # Session management endpoints
    ├── user.rs       # User CRUD
    └── org.rs        # Organization management
```

**Key Features**:
- WebAuthn/Passkeys for passwordless authentication
- OAuth integration (GitHub, with Google/Microsoft planned)
- RBAC with roles: Owner, Admin, Member, Viewer
- Session management via NATS KV

### platform-fleet

Cluster and node management for Craton deployments.

```
platform-fleet/
├── domain/           # Pure functional core
│   ├── cluster.rs    # Cluster aggregates
│   ├── node.rs       # Node aggregates
│   └── decision.rs   # Scheduling decisions
├── agent/            # Agent protocol
│   ├── websocket.rs  # WebSocket handler
│   └── state.rs      # Agent state machine
├── workflows/        # Orchestration
│   ├── create_cluster.rs   # Cluster provisioning
│   └── scale_cluster.rs    # Scaling operations
└── provisioning/     # Node provisioning
    ├── scheduler.rs  # Placement algorithm
    └── health.rs     # Health evaluation
```

**Key Features**:
- Cluster lifecycle management (create, scale, upgrade, delete)
- Agent protocol for node communication
- Health check evaluation engine
- Placement/scheduling algorithms

### platform-nats

NATS JetStream integration for event sourcing and messaging.

```rust
// Event store for domain aggregates
pub trait EventStore {
    fn append(&self, stream: &str, events: Vec<Event>) -> Result<()>;
    fn read(&self, stream: &str, from: u64) -> Result<Vec<Event>>;
}

// Session store using NATS KV
pub trait SessionStore {
    fn create(&self, session: Session) -> Result<()>;
    fn get(&self, id: SessionId) -> Result<Option<Session>>;
    fn delete(&self, id: SessionId) -> Result<()>;
}
```

**Key Features**:
- Event sourcing with JetStream
- Session storage via NATS KV
- Guaranteed message delivery
- Replay capability for projections

### platform-sqlite

SQLite pool for read model projections.

```rust
pub struct SqlitePool { /* ... */ }

impl SqlitePool {
    pub fn query<T>(&self, sql: &str, params: &[Value]) -> Result<Vec<T>>;
    pub fn execute(&self, sql: &str, params: &[Value]) -> Result<u64>;
}
```

**Key Features**:
- Connection pooling
- Automatic migrations
- Checkpoint tracking for event replay

### platform-app

HTTP server and router combining all platform services.

```rust
pub struct AppState {
    pub identity: IdentityService,
    pub fleet: FleetService,
    pub nats: NatsClient,
    pub sqlite: SqlitePool,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .nest("/api/v1/users", user_routes())
        .nest("/api/v1/orgs", org_routes())
        .nest("/api/v1/clusters", cluster_routes())
        .layer(auth_middleware())
        .layer(rate_limit_middleware())
}
```

---

## Data Flow

### Authentication Flow

```
┌────────┐    ┌─────────────────┐    ┌───────────────┐    ┌──────────────┐
│ Client │───▶│ platform-app    │───▶│ platform-     │───▶│ platform-    │
│        │    │ /auth/login     │    │ identity      │    │ nats         │
└────────┘    └─────────────────┘    └───────────────┘    └──────────────┘
                     │                      │                     │
                     │ 1. Validate creds    │ 3. Create session   │
                     │ 2. Check RBAC        │                     │
                     │                      ▼                     ▼
                     │               ┌─────────────┐       ┌─────────────┐
                     │               │ Domain      │       │ NATS KV     │
                     │               │ Decision    │       │ Sessions    │
                     │               └─────────────┘       └─────────────┘
                     ▼
              ┌──────────────┐
              │ JWT Token    │
              │ to Client    │
              └──────────────┘
```

### Cluster Provisioning Flow

```
┌────────┐    ┌─────────────────┐    ┌───────────────┐    ┌──────────────┐
│ Client │───▶│ platform-app    │───▶│ platform-     │───▶│ Scheduler    │
│        │    │ POST /clusters  │    │ fleet         │    │              │
└────────┘    └─────────────────┘    └───────────────┘    └──────────────┘
                                            │                     │
                                            │ 1. Validate quota   │
                                            │ 2. Reserve resources│
                                            │                     ▼
                                            │              ┌─────────────┐
                                            │              │ Node        │
                                            │              │ Selection   │
                                            │              └─────────────┘
                                            ▼                     │
                                     ┌──────────────┐             │
                                     │ Create Event │◀────────────┘
                                     └──────────────┘
                                            │
                                            ▼
                                     ┌──────────────┐
                                     │ NATS         │
                                     │ JetStream    │
                                     └──────────────┘
                                            │
                                            ▼
                                     ┌──────────────┐
                                     │ Agents       │
                                     │ Notified     │
                                     └──────────────┘
```

### Event Sourcing Pattern

The platform uses CQRS (Command Query Responsibility Segregation) with event sourcing:

```
Commands → Workflows → Domain (Pure Decision) → Events → NATS JetStream
                                                              │
                                                              ▼
                                                        SQLite Projections
```

**Write Path**:
1. HTTP request received by `platform-app`
2. Route to appropriate workflow
3. Workflow loads aggregate state from events
4. Domain makes pure decision (Command → Decision → Events)
5. Events appended to NATS JetStream
6. Response returned to client

**Read Path**:
1. Events replayed to SQLite projection on startup
2. Projection updated on each new event
3. Queries served directly from SQLite
4. Eventual consistency (typically < 10ms lag)

---

## Integration with Core Layer

### Current Integration

The platform layer integrates with the core Craton engine via:

1. **craton-client**: RPC client for database operations
2. **craton-wire**: Binary protocol for efficient communication
3. **craton-types**: Shared type definitions

### Planned: platform-data Crate

A new `platform-data` crate will provide:

```rust
pub struct PlatformVdb {
    client: VdbClient,
    tenant_id: TenantId,
}

impl PlatformVdb {
    /// Store platform events in Craton instead of NATS
    pub fn append_event(&self, stream: &str, event: Event) -> Result<Offset>;

    /// Query platform state
    pub fn query(&self, sql: &str, params: &[Value]) -> Result<QueryResult>;

    /// Export platform data for compliance
    pub fn export(&self, options: ExportOptions) -> Result<Export>;
}
```

This migration enables:
- Using Craton's cryptographic guarantees for platform data
- Unified audit trail across platform and tenant data
- Per-tenant database provisioning
- Compliance-grade data export

---

## Security Boundaries

### Tenant Isolation

```
┌─────────────────────────────────────────────────────────────────┐
│                        Platform Layer                            │
│  ┌─────────────────────────────────────────────────────────────┐│
│  │                    Tenant A Context                          ││
│  │  ┌──────────┐  ┌──────────┐  ┌──────────┐                   ││
│  │  │ User A1  │  │ User A2  │  │ Cluster  │                   ││
│  │  └──────────┘  └──────────┘  │ A-prod   │                   ││
│  │                              └──────────┘                    ││
│  └─────────────────────────────────────────────────────────────┘│
│  ┌─────────────────────────────────────────────────────────────┐│
│  │                    Tenant B Context                          ││
│  │  ┌──────────┐  ┌──────────┐                                  ││
│  │  │ User B1  │  │ Cluster  │                                  ││
│  │  └──────────┘  │ B-dev    │                                  ││
│  │               └──────────┘                                   ││
│  └─────────────────────────────────────────────────────────────┘│
└─────────────────────────────────────────────────────────────────┘
```

**Isolation Guarantees**:
- Tenants cannot access each other's data
- RBAC enforced at domain and API layer
- Separate NATS streams per tenant
- Separate Craton tenants per organization

### Authentication Layers

| Layer | Mechanism | Purpose |
|-------|-----------|---------|
| API Gateway | JWT/API Key | Client authentication |
| Service Mesh | mTLS | Service-to-service auth |
| Database | Tenant ID | Data isolation |
| Agent | mTLS + Token | Node authentication |

---

## Operational Considerations

### Scaling

- **Horizontal**: Add more platform-app instances behind load balancer
- **NATS**: Cluster mode for high availability
- **SQLite**: Per-service read replicas (event replay)
- **Craton**: VSR cluster with automatic failover

### Monitoring

Key metrics exposed via Prometheus:
- Request latency (p50, p90, p99)
- Request rate by endpoint
- Error rate by type
- Connection pool utilization
- Event lag (NATS → SQLite projection)

### Backup/Recovery

- **Platform Events**: NATS JetStream with replication
- **Platform State**: SQLite can be rebuilt from events
- **Tenant Data**: Craton checkpoints and exports

---

## Related Documentation

- [DEPLOYMENT.md](DEPLOYMENT.md) - Deployment guide
- [SECURITY.md](SECURITY.md) - Security configuration
- [OPERATIONS.md](OPERATIONS.md) - Operations runbook
- [ARCHITECTURE.md](ARCHITECTURE.md) - Core Craton architecture
