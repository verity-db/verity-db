# Operations Guide

This document describes how to deploy, configure, monitor, and maintain Craton in production environments across any industry requiring verifiable data integrity.

---

## Table of Contents

1. [Deployment Models](#deployment-models)
2. [Configuration](#configuration)
3. [Monitoring and Observability](#monitoring-and-observability)
4. [Backup and Recovery](#backup-and-recovery)
5. [Upgrade Procedures](#upgrade-procedures)
6. [Troubleshooting](#troubleshooting)

---

## Deployment Models

### Single-Node

For development, testing, or small-scale production workloads.

```bash
# Start single-node server
craton-server --data-dir /var/lib/craton --config /etc/craton/config.toml
```

**Characteristics**:
- No replication overhead
- Immediate consistency
- Single point of failure
- Suitable for: dev/test, small workloads, edge deployments

**Directory Structure**:
```
/var/lib/craton/
├── log/                    # Append-only event log
│   ├── 00000000.segment
│   ├── 00000001.segment
│   └── current.segment
├── projections/            # Materialized state
│   ├── tenant_001/
│   └── tenant_002/
├── meta/                   # System metadata
│   └── state.json
└── wal/                    # Write-ahead log (for projections)
```

### Multi-Node Cluster

For production workloads requiring fault tolerance.

```bash
# Node 1 (initial leader)
craton-server \
    --node-id 1 \
    --cluster-peers "node2:7001,node3:7001" \
    --data-dir /var/lib/craton \
    --config /etc/craton/config.toml

# Node 2
craton-server \
    --node-id 2 \
    --cluster-peers "node1:7001,node3:7001" \
    --data-dir /var/lib/craton \
    --config /etc/craton/config.toml

# Node 3
craton-server \
    --node-id 3 \
    --cluster-peers "node1:7001,node2:7001" \
    --data-dir /var/lib/craton \
    --config /etc/craton/config.toml
```

**Cluster Sizes**:

| Nodes | Fault Tolerance | Quorum |
|-------|-----------------|--------|
| 3 | 1 failure | 2 |
| 5 | 2 failures | 3 |
| 7 | 3 failures | 4 |

**Recommendations**:
- Use 3 nodes for most workloads
- Use 5 nodes for critical data with higher availability requirements
- Deploy nodes in different availability zones when possible

### Multi-Region

For global deployments with data residency requirements.

```
┌─────────────────────┐     ┌─────────────────────┐
│    US-East Region   │     │    EU-West Region   │
│                     │     │                     │
│  ┌───────────────┐  │     │  ┌───────────────┐  │
│  │ 3-node cluster│  │     │  │ 3-node cluster│  │
│  │ (US tenants)  │  │     │  │ (EU tenants)  │  │
│  └───────────────┘  │     │  └───────────────┘  │
└─────────────────────┘     └─────────────────────┘
         │                           │
         └───────────┬───────────────┘
                     │
              ┌──────────────┐
              │   Directory  │
              │   Service    │
              └──────────────┘
```

Each region is an independent cluster. Tenants are assigned to regions based on data residency requirements (regulatory, contractual, or organizational), and their data never leaves that region.

---

## Configuration

### Configuration File

```toml
# /etc/craton/config.toml

[server]
# Network binding
bind_address = "0.0.0.0:7000"
cluster_address = "0.0.0.0:7001"

# Node identity
node_id = 1
cluster_name = "production"

[storage]
# Data directory (absolute path required)
data_dir = "/var/lib/craton"

# Segment size (default 64MB)
segment_size = "64MB"

# Sync mode: "fsync" (durable) or "async" (faster, less safe)
sync_mode = "fsync"

[consensus]
# Heartbeat interval
heartbeat_interval = "100ms"

# Election timeout (min, max)
election_timeout_min = "150ms"
election_timeout_max = "300ms"

# Maximum entries per AppendEntries RPC
max_entries_per_rpc = 100

[projections]
# Maximum memory for projection cache
cache_size = "1GB"

# Checkpoint interval (by log entries)
checkpoint_interval = 10000

[security]
# TLS configuration
tls_cert = "/etc/craton/server.crt"
tls_key = "/etc/craton/server.key"
tls_ca = "/etc/craton/ca.crt"

# Require client certificates (mutual TLS)
require_client_cert = true

[encryption]
# Enable at-rest encryption
enabled = true

# KMS provider: "local" or "aws-kms" or "gcp-kms" or "azure-keyvault"
kms_provider = "aws-kms"
kms_key_id = "arn:aws:kms:us-east-1:123456789:key/abc123"

[limits]
# Per-tenant limits
max_tenants = 1000
max_streams_per_tenant = 100
max_record_size = "1MB"
max_batch_size = "10MB"

[telemetry]
# Metrics endpoint
metrics_address = "0.0.0.0:9090"

# Tracing
tracing_enabled = true
tracing_endpoint = "http://jaeger:14268/api/traces"
```

### Environment Variables

Configuration can be overridden via environment variables:

```bash
CRATON_NODE_ID=1
CRATON_BIND_ADDRESS=0.0.0.0:7000
CRATON_DATA_DIR=/var/lib/craton
CRATON_LOG_LEVEL=info
```

### CLI Flags

```bash
craton-server --help

Usage: craton-server [OPTIONS]

Options:
    --config <PATH>          Configuration file path
    --node-id <ID>           Node identifier (overrides config)
    --bind <ADDR>            Bind address (overrides config)
    --data-dir <PATH>        Data directory (overrides config)
    --cluster-peers <PEERS>  Comma-separated peer addresses
    --log-level <LEVEL>      Log level: trace, debug, info, warn, error
    -h, --help               Print help
    -V, --version            Print version
```

---

## Monitoring and Observability

### Metrics

Craton exposes Prometheus-compatible metrics:

```bash
curl http://localhost:9090/metrics
```

**Key Metrics**:

| Metric | Type | Description |
|--------|------|-------------|
| `craton_log_entries_total` | Counter | Total log entries written |
| `craton_log_bytes_total` | Counter | Total log bytes written |
| `craton_consensus_commits_total` | Counter | Total committed entries |
| `craton_consensus_view` | Gauge | Current consensus view |
| `craton_consensus_leader` | Gauge | Current leader node ID |
| `craton_projection_applied_position` | Gauge | Last applied log position |
| `craton_query_duration_seconds` | Histogram | Query latency distribution |
| `craton_write_duration_seconds` | Histogram | Write latency distribution |
| `craton_active_tenants` | Gauge | Number of active tenants |

**Server Metrics** (craton-server):

| Metric | Type | Description |
|--------|------|-------------|
| `vdb_requests_total` | Counter | Total requests by method and status |
| `vdb_request_duration_seconds` | Histogram | Request latency distribution |
| `vdb_requests_in_flight` | Gauge | Currently processing requests |
| `vdb_connections_total` | Counter | Total connections accepted |
| `vdb_connections_active` | Gauge | Currently active connections |
| `vdb_errors_total` | Counter | Errors by type |
| `vdb_rate_limited_total` | Counter | Requests rejected by rate limiting |
| `vdb_storage_bytes_written_total` | Counter | Bytes written to storage |
| `vdb_storage_records_total` | Counter | Records written to storage |
| `vdb_storage_checkpoints_total` | Counter | Checkpoints created |
| `vdb_auth_attempts_total` | Counter | Auth attempts by method and result |

**Example Prometheus Configuration**:

```yaml
scrape_configs:
  - job_name: 'craton'
    static_configs:
      - targets:
        - 'craton-node1:9090'
        - 'craton-node2:9090'
        - 'craton-node3:9090'
```

### Logging

Structured JSON logging:

```json
{
  "timestamp": "2024-01-15T10:30:00.000Z",
  "level": "INFO",
  "target": "craton_consensus",
  "message": "Became leader",
  "fields": {
    "node_id": 1,
    "view": 5,
    "commit_index": 12345
  }
}
```

**Log Levels**:
- `error`: Errors requiring attention
- `warn`: Unexpected but recoverable situations
- `info`: Normal operational messages
- `debug`: Detailed debugging information
- `trace`: Very verbose, for development only

### Tracing

Distributed tracing with OpenTelemetry:

```rust
// Spans are automatically created for key operations
// - craton.write: Write path from client to commit
// - craton.query: Query execution
// - craton.consensus.prepare: Consensus prepare phase
// - craton.consensus.commit: Consensus commit phase
```

### Health Checks

The craton-server provides HTTP endpoints for health monitoring:

```bash
# Liveness (is the process running?)
curl http://localhost:5432/health

# Readiness (can it serve requests?)
curl http://localhost:5432/ready

# Prometheus metrics
curl http://localhost:5432/metrics
```

**Liveness Response** (`/health`):
```json
{
  "status": "ok",
  "version": "0.1.0",
  "uptime_seconds": 3600
}
```

**Readiness Response** (`/ready`) - includes dependency checks:
```json
{
  "status": "ok",
  "checks": {
    "disk": {"status": "ok", "duration_ms": 1},
    "memory": {"status": "ok", "duration_ms": 0},
    "data_dir": {"status": "ok", "duration_ms": 2}
  },
  "version": "0.1.0",
  "uptime_seconds": 3600
}
```

**Health Status Values**:
- `ok` - Service is healthy
- `degraded` - Service is functional but with warnings (e.g., high connection count >900)
- `unhealthy` - Service cannot handle requests (e.g., data directory not writable)

**Kubernetes Configuration**:
```yaml
livenessProbe:
  httpGet:
    path: /health
    port: 5432
  initialDelaySeconds: 10
  periodSeconds: 10
readinessProbe:
  httpGet:
    path: /ready
    port: 5432
  initialDelaySeconds: 5
  periodSeconds: 5
```

**Status Response**:
```json
{
  "node_id": 1,
  "state": "leader",
  "view": 5,
  "commit_index": 12345,
  "applied_index": 12345,
  "cluster": {
    "peers": [
      {"id": 2, "state": "follower", "last_seen": "2024-01-15T10:30:00Z"},
      {"id": 3, "state": "follower", "last_seen": "2024-01-15T10:30:00Z"}
    ]
  }
}
```

---

## Backup and Recovery

### Continuous Backup

Craton's append-only log makes continuous backup straightforward:

```bash
# Stream log segments to backup storage as they're completed
craton-backup stream \
    --source /var/lib/craton/log \
    --destination s3://craton-backups/production/
```

### Point-in-Time Backup

```bash
# Create consistent backup at specific position
craton-backup snapshot \
    --data-dir /var/lib/craton \
    --position 12345 \
    --output /backups/craton-12345.tar.gz
```

### Restore

```bash
# Restore from backup
craton-restore \
    --source /backups/craton-12345.tar.gz \
    --data-dir /var/lib/craton

# Verify restored data
craton-admin verify --data-dir /var/lib/craton
```

### Backup Strategy Recommendations

| Strategy | RPO | Use Case |
|----------|-----|----------|
| Continuous replication | ~0 | Production, multi-node |
| Hourly segment backup | 1 hour | Cost-effective backup |
| Daily full backup | 24 hours | Disaster recovery |

---

## Upgrade Procedures

### Rolling Upgrade

For multi-node clusters, upgrade one node at a time:

```bash
# 1. Check cluster health
craton-admin cluster status

# 2. Upgrade follower nodes first
# On node 2:
sudo systemctl stop craton
# Install new version
sudo systemctl start craton
# Wait for node to rejoin and sync
craton-admin cluster wait-healthy

# On node 3:
# (same process)

# 3. Upgrade leader last
# Current leader will step down automatically
# On node 1:
sudo systemctl stop craton
# Install new version
sudo systemctl start craton
```

### Schema Migrations

```bash
# Apply schema migration
craton-admin migrate \
    --tenant 123 \
    --migration-file /migrations/001_add_column.sql

# Verify migration
craton-admin schema show --tenant 123
```

### Rollback

If issues arise:

```bash
# 1. Stop the problematic node
sudo systemctl stop craton

# 2. Restore previous version
sudo dpkg -i craton-server-previous.deb

# 3. Start with previous version
sudo systemctl start craton
```

---

## Troubleshooting

### Common Issues

**Cluster Won't Form**

```bash
# Check connectivity
craton-admin cluster ping --peers node1:7001,node2:7001,node3:7001

# Check logs for connection errors
journalctl -u craton -f | grep -i "connection\|timeout\|refused"

# Verify firewall rules
# Port 7000: Client traffic
# Port 7001: Cluster traffic
```

**High Write Latency**

```bash
# Check disk I/O
iostat -x 1

# Check consensus timing
craton-admin metrics | grep consensus_duration

# Check if syncing
craton-admin status | grep -i sync
```

**Projection Lag**

```bash
# Check lag between log and projections
craton-admin status | grep -E "(commit_index|applied_index)"

# Force projection rebuild if corrupted
craton-admin projection rebuild --tenant 123 --projection records
```

**Node Won't Start**

```bash
# Check configuration
craton-server --config /etc/craton/config.toml --validate

# Check data directory permissions
ls -la /var/lib/craton

# Check for corrupted state
craton-admin verify --data-dir /var/lib/craton
```

### Diagnostic Commands

```bash
# Cluster status
craton-admin cluster status

# Node status
craton-admin node status

# Log integrity check
craton-admin log verify --data-dir /var/lib/craton

# Projection status
craton-admin projection status --tenant 123

# Export diagnostics bundle
craton-admin diagnostics export --output /tmp/craton-diag.tar.gz
```

### Recovery Procedures

**Single Node Failure (Multi-Node Cluster)**

1. Cluster continues operating with remaining nodes
2. Replace failed node hardware/VM
3. Start new node with same node-id
4. Node automatically syncs from healthy peers

**Majority Failure**

1. **STOP**: Do not attempt to bring up nodes independently
2. Identify node with highest commit index
3. Start that node first with `--force-recover`
4. Start other nodes to rejoin

**Data Corruption**

1. Stop affected node
2. Verify backup integrity
3. Restore from backup
4. Rejoin cluster (will sync missing entries)

---

## Summary

Operating Craton in production:

1. **Start simple**: Begin with single-node, scale to multi-node as needed
2. **Monitor everything**: Metrics, logs, and health checks are essential
3. **Backup continuously**: The append-only log makes this easy
4. **Upgrade carefully**: Rolling upgrades minimize risk
5. **Plan for failure**: Know the recovery procedures before you need them

For additional support:
- Documentation: https://docs.craton.io
- Community: https://github.com/craton/craton/discussions
- Enterprise support: support@craton.io
