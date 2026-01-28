# Craton Agent Protocol

This document describes the protocol for communication between Craton cluster agents and control plane systems.

## Overview

The agent protocol enables:

- **Health monitoring**: Agents report node status via heartbeats
- **Configuration management**: Control planes push configuration updates
- **Observability**: Metrics and logs are streamed from agents
- **Administration**: Control planes can issue administrative commands

## Transport

The protocol uses WebSocket connections with JSON-encoded messages. Agents connect to the control plane endpoint and maintain a persistent connection.

```
Agent                          Control Plane
  |                                  |
  |-------- WebSocket Connect ------>|
  |                                  |
  |<------- HeartbeatRequest --------|
  |-------- Heartbeat ------------->|
  |                                  |
  |-------- MetricsBatch ---------->|
  |-------- LogsBatch ------------->|
  |                                  |
  |<------- ConfigUpdate -----------|
  |-------- ConfigAck ------------->|
  |                                  |
```

## Message Types

### Agent → Control Plane

#### Heartbeat

Periodic status update sent by the agent.

```json
{
  "type": "heartbeat",
  "node_id": "node-001",
  "status": "healthy",
  "role": "leader",
  "resources": {
    "cpu_percent": 45.2,
    "memory_used_bytes": 1073741824,
    "memory_total_bytes": 8589934592,
    "disk_used_bytes": 10737418240,
    "disk_total_bytes": 107374182400
  },
  "replication": null
}
```

**Fields:**

| Field | Type | Description |
|-------|------|-------------|
| `node_id` | string | Unique identifier for the node |
| `status` | enum | `healthy`, `degraded`, `unhealthy`, `starting`, `stopping` |
| `role` | enum | `leader`, `follower`, `candidate`, `learner` |
| `resources` | object | Current resource utilization |
| `replication` | object? | Replication status (followers only) |

**Replication object:**

| Field | Type | Description |
|-------|------|-------------|
| `leader_id` | string | ID of the leader being replicated from |
| `lag_ms` | u64 | Replication lag in milliseconds |
| `pending_entries` | u64 | Number of entries waiting to replicate |

#### MetricsBatch

Batch of collected metric samples.

```json
{
  "type": "metrics_batch",
  "node_id": "node-001",
  "metrics": [
    {
      "name": "craton.writes.total",
      "value": 12345.0,
      "timestamp_ms": 1700000000000,
      "labels": [["tenant", "acme"]]
    }
  ]
}
```

**Metric sample fields:**

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Metric name (e.g., `craton.writes.total`) |
| `value` | f64 | Metric value |
| `timestamp_ms` | u64 | Unix timestamp in milliseconds |
| `labels` | array | Optional key-value pairs |

#### LogsBatch

Batch of log entries.

```json
{
  "type": "logs_batch",
  "node_id": "node-001",
  "entries": [
    {
      "timestamp_ms": 1700000000000,
      "level": "info",
      "message": "Snapshot completed",
      "fields": [["duration_ms", "1234"]]
    }
  ]
}
```

**Log entry fields:**

| Field | Type | Description |
|-------|------|-------------|
| `timestamp_ms` | u64 | Unix timestamp in milliseconds |
| `level` | enum | `trace`, `debug`, `info`, `warn`, `error` |
| `message` | string | Log message |
| `fields` | array | Optional structured fields |

#### ConfigAck

Acknowledgment of a configuration update.

```json
{
  "type": "config_ack",
  "version": 42,
  "success": true,
  "error": null
}
```

**Fields:**

| Field | Type | Description |
|-------|------|-------------|
| `version` | u64 | Configuration version being acknowledged |
| `success` | bool | Whether configuration was applied |
| `error` | string? | Error message if failed |

### Control Plane → Agent

#### ConfigUpdate

Push new configuration to the agent.

```json
{
  "type": "config_update",
  "version": 42,
  "config": "{\"max_connections\": 100}",
  "checksum": "sha256:abc123..."
}
```

**Fields:**

| Field | Type | Description |
|-------|------|-------------|
| `version` | u64 | Configuration version |
| `config` | string | JSON-encoded configuration |
| `checksum` | string | Integrity checksum |

The agent should verify the checksum before applying the configuration and respond with a `ConfigAck`.

#### AdminCommand

Execute an administrative command.

```json
{
  "type": "admin_command",
  "command": "take_snapshot"
}
```

**Available commands:**

| Command | Description |
|---------|-------------|
| `take_snapshot` | Trigger a state snapshot |
| `compact_log` | Compact log up to offset (`up_to_offset` field) |
| `step_down` | Step down from leader role |
| `transfer_leadership` | Transfer to target (`target_node_id` field) |
| `pause_replication` | Pause replication for maintenance |
| `resume_replication` | Resume replication |

#### HeartbeatRequest

Request an immediate heartbeat from the agent.

```json
{
  "type": "heartbeat_request"
}
```

#### Shutdown

Request graceful shutdown.

```json
{
  "type": "shutdown",
  "reason": "Cluster scaling down"
}
```

## Connection Lifecycle

### Initial Connection

1. Agent connects to WebSocket endpoint
2. Control plane sends `HeartbeatRequest`
3. Agent responds with `Heartbeat`
4. Connection is established

### Steady State

- Agent sends `Heartbeat` every 10 seconds (configurable)
- Agent batches and sends `MetricsBatch` every 5 seconds
- Agent batches and sends `LogsBatch` every 5 seconds
- Control plane pushes `ConfigUpdate` as needed

### Reconnection

If the connection drops, agents should:

1. Wait with exponential backoff (initial: 1s, max: 60s)
2. Reconnect and perform initial handshake
3. Resume normal operation

## Using the Protocol

### Rust Crate

The `craton-agent-protocol` crate provides typed definitions:

```rust
use vdb_agent_protocol::{AgentMessage, NodeStatus, NodeRole, Resources};

let heartbeat = AgentMessage::Heartbeat {
    node_id: "node-001".to_string(),
    status: NodeStatus::Healthy,
    role: NodeRole::Leader,
    resources: Resources {
        cpu_percent: 45.2,
        memory_used_bytes: 1_073_741_824,
        memory_total_bytes: 8_589_934_592,
        disk_used_bytes: 10_737_418_240,
        disk_total_bytes: 107_374_182_400,
    },
    replication: None,
};

let json = serde_json::to_string(&heartbeat)?;
```

### Other Languages

The protocol uses standard JSON, so any language with JSON support can implement an agent or control plane. The type definitions in this document serve as the canonical specification.

## Versioning

The protocol version is negotiated during the WebSocket handshake via the `Sec-WebSocket-Protocol` header:

```
Sec-WebSocket-Protocol: craton-agent-protocol-v1
```

Breaking changes will increment the version number.
