//! Agent protocol types for `Craton` cluster management.
//!
//! This crate defines the protocol used for communication between `Craton`
//! cluster agents and control plane systems. It is designed to be used by:
//!
//! - **Self-hosters**: Building custom agents or control planes
//! - **Platform operators**: The official `Craton` platform
//! - **Tooling authors**: Monitoring, automation, and integration tools
//!
//! # Protocol Overview
//!
//! The protocol uses a request-response pattern over WebSocket connections.
//! Agents connect to the control plane and exchange typed messages.
//!
//! ## Agent → Control Plane
//!
//! - [`AgentMessage::Heartbeat`] - Periodic health and status updates
//! - [`AgentMessage::MetricsBatch`] - Collected metrics samples
//! - [`AgentMessage::LogsBatch`] - Log entries from the node
//! - [`AgentMessage::ConfigAck`] - Acknowledgment of configuration changes
//!
//! ## Control Plane → Agent
//!
//! - [`ControlMessage::ConfigUpdate`] - Push new configuration
//! - [`ControlMessage::AdminCommand`] - Administrative operations
//! - [`ControlMessage::HeartbeatRequest`] - Request immediate status
//! - [`ControlMessage::Shutdown`] - Graceful shutdown request
//!
//! # Example
//!
//! ```rust
//! use craton_agent_protocol::{AgentMessage, NodeStatus, NodeRole, Resources};
//!
//! let heartbeat = AgentMessage::Heartbeat {
//!     node_id: "node-001".to_string(),
//!     status: NodeStatus::Healthy,
//!     role: NodeRole::Leader,
//!     resources: Resources {
//!         cpu_percent: 45.2,
//!         memory_used_bytes: 1_073_741_824,
//!         memory_total_bytes: 8_589_934_592,
//!         disk_used_bytes: 10_737_418_240,
//!         disk_total_bytes: 107_374_182_400,
//!     },
//!     replication: None,
//! };
//! ```

use serde::{Deserialize, Serialize};

// ============================================================================
// Core Identifiers
// ============================================================================

/// Unique identifier for a `Craton` cluster.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ClusterId(pub String);

/// Unique identifier for a node within a cluster.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(pub String);

/// Configuration version identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ConfigVersion(pub u64);

// ============================================================================
// Agent → Control Plane Messages
// ============================================================================

/// Messages sent from an agent to the control plane.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentMessage {
    /// Periodic heartbeat with node status and resource usage.
    Heartbeat {
        /// The node sending this heartbeat.
        node_id: String,
        /// Current health status of the node.
        status: NodeStatus,
        /// Current role in the cluster.
        role: NodeRole,
        /// Resource utilization snapshot.
        resources: Resources,
        /// Replication status (if applicable).
        replication: Option<ReplicationStatus>,
    },

    /// Batch of collected metrics samples.
    MetricsBatch {
        /// The node these metrics are from.
        node_id: String,
        /// Collected metric samples.
        metrics: Vec<MetricSample>,
    },

    /// Batch of log entries.
    LogsBatch {
        /// The node these logs are from.
        node_id: String,
        /// Log entries.
        entries: Vec<LogEntry>,
    },

    /// Acknowledgment of a configuration update.
    ConfigAck {
        /// The configuration version being acknowledged.
        version: ConfigVersion,
        /// Whether the configuration was applied successfully.
        success: bool,
        /// Error message if the configuration failed to apply.
        error: Option<String>,
    },
}

/// Health status of a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    /// Node is operating normally.
    Healthy,
    /// Node is experiencing issues but still operational.
    Degraded,
    /// Node is not operational.
    Unhealthy,
    /// Node is starting up.
    Starting,
    /// Node is shutting down.
    Stopping,
}

/// Role of a node in the cluster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeRole {
    /// Primary node handling writes.
    Leader,
    /// Secondary node replicating from the leader.
    Follower,
    /// Node participating in leader election.
    Candidate,
    /// Node is not yet part of the cluster.
    Learner,
}

/// Resource utilization snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resources {
    /// CPU utilization as a percentage (0.0 - 100.0).
    pub cpu_percent: f64,
    /// Memory currently used in bytes.
    pub memory_used_bytes: u64,
    /// Total memory available in bytes.
    pub memory_total_bytes: u64,
    /// Disk space used in bytes.
    pub disk_used_bytes: u64,
    /// Total disk space in bytes.
    pub disk_total_bytes: u64,
}

/// Replication status for a follower node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicationStatus {
    /// ID of the leader being replicated from.
    pub leader_id: String,
    /// Replication lag in milliseconds.
    pub lag_ms: u64,
    /// Number of pending entries to replicate.
    pub pending_entries: u64,
}

/// A single metric sample.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricSample {
    /// Metric name (e.g., "craton.writes.total").
    pub name: String,
    /// Metric value.
    pub value: f64,
    /// Unix timestamp in milliseconds.
    pub timestamp_ms: u64,
    /// Optional labels for the metric.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<(String, String)>,
}

/// A log entry from a node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    /// Unix timestamp in milliseconds.
    pub timestamp_ms: u64,
    /// Log level.
    pub level: LogLevel,
    /// Log message.
    pub message: String,
    /// Optional structured fields.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<(String, String)>,
}

/// Log secraton level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

// ============================================================================
// Control Plane → Agent Messages
// ============================================================================

/// Messages sent from the control plane to an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControlMessage {
    /// Push a new configuration to the agent.
    ConfigUpdate {
        /// Version of this configuration.
        version: ConfigVersion,
        /// Configuration content (JSON-encoded).
        config: String,
        /// Checksum for integrity verification.
        checksum: String,
    },

    /// Execute an administrative command.
    AdminCommand(AdminCommand),

    /// Request an immediate heartbeat.
    HeartbeatRequest,

    /// Request graceful shutdown.
    Shutdown {
        /// Reason for the shutdown.
        reason: String,
    },
}

/// Administrative commands that can be sent to an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum AdminCommand {
    /// Trigger a snapshot of the current state.
    TakeSnapshot,
    /// Compact the log up to a given offset.
    CompactLog { up_to_offset: u64 },
    /// Step down from leader role (if leader).
    StepDown,
    /// Transfer leadership to a specific node.
    TransferLeadership { target_node_id: String },
    /// Pause replication (for maintenance).
    PauseReplication,
    /// Resume replication.
    ResumeReplication,
}

// ============================================================================
// Error Types
// ============================================================================

/// Errors that can occur during protocol operations.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ProtocolError {
    /// Failed to serialize a message.
    #[error("serialization failed: {0}")]
    Serialization(String),

    /// Failed to deserialize a message.
    #[error("deserialization failed: {0}")]
    Deserialization(String),

    /// Invalid message format.
    #[error("invalid message: {0}")]
    InvalidMessage(String),

    /// Configuration checksum mismatch.
    #[error("checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_roundtrip() {
        let msg = AgentMessage::Heartbeat {
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

        let json = serde_json::to_string(&msg).expect("serialize");
        let decoded: AgentMessage = serde_json::from_str(&json).expect("deserialize");

        match decoded {
            AgentMessage::Heartbeat {
                node_id, status, ..
            } => {
                assert_eq!(node_id, "node-001");
                assert_eq!(status, NodeStatus::Healthy);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn config_update_roundtrip() {
        let msg = ControlMessage::ConfigUpdate {
            version: ConfigVersion(42),
            config: r#"{"max_connections": 100}"#.to_string(),
            checksum: "sha256:abc123".to_string(),
        };

        let json = serde_json::to_string(&msg).expect("serialize");
        let decoded: ControlMessage = serde_json::from_str(&json).expect("deserialize");

        match decoded {
            ControlMessage::ConfigUpdate { version, .. } => {
                assert_eq!(version, ConfigVersion(42));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn admin_command_variants() {
        let commands = vec![
            AdminCommand::TakeSnapshot,
            AdminCommand::CompactLog { up_to_offset: 1000 },
            AdminCommand::StepDown,
            AdminCommand::TransferLeadership {
                target_node_id: "node-002".to_string(),
            },
            AdminCommand::PauseReplication,
            AdminCommand::ResumeReplication,
        ];

        for cmd in commands {
            let msg = ControlMessage::AdminCommand(cmd);
            let json = serde_json::to_string(&msg).expect("serialize");
            let _decoded: ControlMessage = serde_json::from_str(&json).expect("deserialize");
        }
    }

    #[test]
    fn metrics_batch_roundtrip() {
        let msg = AgentMessage::MetricsBatch {
            node_id: "node-001".to_string(),
            metrics: vec![
                MetricSample {
                    name: "craton.writes.total".to_string(),
                    value: 12345.0,
                    timestamp_ms: 1700000000000,
                    labels: vec![("tenant".to_string(), "acme".to_string())],
                },
                MetricSample {
                    name: "craton.reads.total".to_string(),
                    value: 98765.0,
                    timestamp_ms: 1700000000000,
                    labels: vec![],
                },
            ],
        };

        let json = serde_json::to_string(&msg).expect("serialize");
        let decoded: AgentMessage = serde_json::from_str(&json).expect("deserialize");

        match decoded {
            AgentMessage::MetricsBatch { metrics, .. } => {
                assert_eq!(metrics.len(), 2);
            }
            _ => panic!("wrong variant"),
        }
    }
}
