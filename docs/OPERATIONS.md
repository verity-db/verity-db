# Operations Guide

This document describes how to deploy, configure, monitor, and maintain VerityDB in production environments.

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
verity-server --data-dir /var/lib/verity --config /etc/verity/config.toml
```

**Characteristics**:
- No replication overhead
- Immediate consistency
- Single point of failure
- Suitable for: dev/test, small workloads, edge deployments

**Directory Structure**:
```
/var/lib/verity/
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
verity-server \
    --node-id 1 \
    --cluster-peers "node2:7001,node3:7001" \
    --data-dir /var/lib/verity \
    --config /etc/verity/config.toml

# Node 2
verity-server \
    --node-id 2 \
    --cluster-peers "node1:7001,node3:7001" \
    --data-dir /var/lib/verity \
    --config /etc/verity/config.toml

# Node 3
verity-server \
    --node-id 3 \
    --cluster-peers "node1:7001,node2:7001" \
    --data-dir /var/lib/verity \
    --config /etc/verity/config.toml
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

Each region is an independent cluster. Tenants are assigned to regions and their data never leaves that region.

---

## Configuration

### Configuration File

```toml
# /etc/verity/config.toml

[server]
# Network binding
bind_address = "0.0.0.0:7000"
cluster_address = "0.0.0.0:7001"

# Node identity
node_id = 1
cluster_name = "production"

[storage]
# Data directory (absolute path required)
data_dir = "/var/lib/verity"

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
tls_cert = "/etc/verity/server.crt"
tls_key = "/etc/verity/server.key"
tls_ca = "/etc/verity/ca.crt"

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
VERITY_NODE_ID=1
VERITY_BIND_ADDRESS=0.0.0.0:7000
VERITY_DATA_DIR=/var/lib/verity
VERITY_LOG_LEVEL=info
```

### CLI Flags

```bash
verity-server --help

Usage: verity-server [OPTIONS]

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

VerityDB exposes Prometheus-compatible metrics:

```bash
curl http://localhost:9090/metrics
```

**Key Metrics**:

| Metric | Type | Description |
|--------|------|-------------|
| `verity_log_entries_total` | Counter | Total log entries written |
| `verity_log_bytes_total` | Counter | Total log bytes written |
| `verity_consensus_commits_total` | Counter | Total committed entries |
| `verity_consensus_view` | Gauge | Current consensus view |
| `verity_consensus_leader` | Gauge | Current leader node ID |
| `verity_projection_applied_position` | Gauge | Last applied log position |
| `verity_query_duration_seconds` | Histogram | Query latency distribution |
| `verity_write_duration_seconds` | Histogram | Write latency distribution |
| `verity_active_tenants` | Gauge | Number of active tenants |

**Example Prometheus Configuration**:

```yaml
scrape_configs:
  - job_name: 'verity'
    static_configs:
      - targets:
        - 'verity-node1:9090'
        - 'verity-node2:9090'
        - 'verity-node3:9090'
```

### Logging

Structured JSON logging:

```json
{
  "timestamp": "2024-01-15T10:30:00.000Z",
  "level": "INFO",
  "target": "verity_consensus",
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
// - verity.write: Write path from client to commit
// - verity.query: Query execution
// - verity.consensus.prepare: Consensus prepare phase
// - verity.consensus.commit: Consensus commit phase
```

### Health Checks

```bash
# Liveness (is the process running?)
curl http://localhost:7000/health/live

# Readiness (can it serve requests?)
curl http://localhost:7000/health/ready

# Detailed status
curl http://localhost:7000/status
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

VerityDB's append-only log makes continuous backup straightforward:

```bash
# Stream log segments to backup storage as they're completed
verity-backup stream \
    --source /var/lib/verity/log \
    --destination s3://verity-backups/production/
```

### Point-in-Time Backup

```bash
# Create consistent backup at specific position
verity-backup snapshot \
    --data-dir /var/lib/verity \
    --position 12345 \
    --output /backups/verity-12345.tar.gz
```

### Restore

```bash
# Restore from backup
verity-restore \
    --source /backups/verity-12345.tar.gz \
    --data-dir /var/lib/verity

# Verify restored data
verity-admin verify --data-dir /var/lib/verity
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
verity-admin cluster status

# 2. Upgrade follower nodes first
# On node 2:
sudo systemctl stop verity
# Install new version
sudo systemctl start verity
# Wait for node to rejoin and sync
verity-admin cluster wait-healthy

# On node 3:
# (same process)

# 3. Upgrade leader last
# Current leader will step down automatically
# On node 1:
sudo systemctl stop verity
# Install new version
sudo systemctl start verity
```

### Schema Migrations

```bash
# Apply schema migration
verity-admin migrate \
    --tenant 123 \
    --migration-file /migrations/001_add_column.sql

# Verify migration
verity-admin schema show --tenant 123
```

### Rollback

If issues arise:

```bash
# 1. Stop the problematic node
sudo systemctl stop verity

# 2. Restore previous version
sudo dpkg -i verity-server-previous.deb

# 3. Start with previous version
sudo systemctl start verity
```

---

## Troubleshooting

### Common Issues

**Cluster Won't Form**

```bash
# Check connectivity
verity-admin cluster ping --peers node1:7001,node2:7001,node3:7001

# Check logs for connection errors
journalctl -u verity -f | grep -i "connection\|timeout\|refused"

# Verify firewall rules
# Port 7000: Client traffic
# Port 7001: Cluster traffic
```

**High Write Latency**

```bash
# Check disk I/O
iostat -x 1

# Check consensus timing
verity-admin metrics | grep consensus_duration

# Check if syncing
verity-admin status | grep -i sync
```

**Projection Lag**

```bash
# Check lag between log and projections
verity-admin status | grep -E "(commit_index|applied_index)"

# Force projection rebuild if corrupted
verity-admin projection rebuild --tenant 123 --projection patients
```

**Node Won't Start**

```bash
# Check configuration
verity-server --config /etc/verity/config.toml --validate

# Check data directory permissions
ls -la /var/lib/verity

# Check for corrupted state
verity-admin verify --data-dir /var/lib/verity
```

### Diagnostic Commands

```bash
# Cluster status
verity-admin cluster status

# Node status
verity-admin node status

# Log integrity check
verity-admin log verify --data-dir /var/lib/verity

# Projection status
verity-admin projection status --tenant 123

# Export diagnostics bundle
verity-admin diagnostics export --output /tmp/verity-diag.tar.gz
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

Operating VerityDB in production:

1. **Start simple**: Begin with single-node, scale to multi-node as needed
2. **Monitor everything**: Metrics, logs, and health checks are essential
3. **Backup continuously**: The append-only log makes this easy
4. **Upgrade carefully**: Rolling upgrades minimize risk
5. **Plan for failure**: Know the recovery procedures before you need them

For additional support:
- Documentation: https://docs.veritydb.io
- Community: https://github.com/veritydb/veritydb/discussions
- Enterprise support: support@veritydb.io
