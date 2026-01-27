//! Simulated network for deterministic testing.
//!
//! The `SimNetwork` models an unreliable network with configurable:
//! - Message delays (latency)
//! - Message loss
//! - Network partitions
//! - Message reordering
//!
//! All behavior is deterministic based on the simulation's RNG seed.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::rng::SimRng;

// ============================================================================
// Network Configuration
// ============================================================================

/// Configuration for simulated network behavior.
#[derive(Debug, Clone)]
pub struct NetworkConfig {
    /// Minimum message delay in nanoseconds.
    pub min_delay_ns: u64,
    /// Maximum message delay in nanoseconds.
    pub max_delay_ns: u64,
    /// Probability of dropping a message (0.0 to 1.0).
    pub drop_probability: f64,
    /// Probability of duplicating a message (0.0 to 1.0).
    pub duplicate_probability: f64,
    /// Maximum number of in-flight messages per link.
    pub max_in_flight: usize,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            min_delay_ns: 100_000,       // 0.1ms
            max_delay_ns: 10_000_000,    // 10ms
            drop_probability: 0.0,       // No drops by default
            duplicate_probability: 0.0,  // No duplicates by default
            max_in_flight: 10_000,
        }
    }
}

impl NetworkConfig {
    /// Creates a reliable network configuration (no drops, no duplicates).
    pub fn reliable() -> Self {
        Self::default()
    }

    /// Creates a lossy network configuration.
    pub fn lossy(drop_probability: f64) -> Self {
        Self {
            drop_probability,
            ..Self::default()
        }
    }

    /// Creates a high-latency network configuration.
    pub fn high_latency(min_delay_ns: u64, max_delay_ns: u64) -> Self {
        Self {
            min_delay_ns,
            max_delay_ns,
            ..Self::default()
        }
    }
}

// ============================================================================
// Message Types
// ============================================================================

/// Unique identifier for a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MessageId(u64);

impl MessageId {
    /// Creates a message ID from a raw value.
    pub fn from_raw(id: u64) -> Self {
        Self(id)
    }

    /// Returns the raw message ID.
    pub fn as_raw(&self) -> u64 {
        self.0
    }
}

/// A network message in transit.
#[derive(Debug, Clone)]
pub struct Message {
    /// Unique message identifier.
    pub id: MessageId,
    /// Source node ID.
    pub from: u64,
    /// Destination node ID.
    pub to: u64,
    /// Message payload (opaque bytes).
    pub payload: Vec<u8>,
    /// When the message was sent (simulation time).
    pub sent_at_ns: u64,
    /// When the message should be delivered (simulation time).
    pub deliver_at_ns: u64,
}

/// Result of attempting to send a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendResult {
    /// Message was queued for delivery.
    Queued { message_id: MessageId, deliver_at_ns: u64 },
    /// Message was dropped (simulated loss).
    Dropped,
    /// Message was rejected (queue full or partition).
    Rejected { reason: RejectReason },
}

/// Reason a message was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RejectReason {
    /// The destination is unreachable due to partition.
    Partitioned,
    /// The message queue is full.
    QueueFull,
    /// The destination node doesn't exist.
    UnknownDestination,
}

// ============================================================================
// Partition Management
// ============================================================================

/// Represents a network partition.
#[derive(Debug, Clone)]
pub struct Partition {
    /// Unique partition identifier.
    pub id: u64,
    /// Nodes in partition group A.
    pub group_a: HashSet<u64>,
    /// Nodes in partition group B.
    pub group_b: HashSet<u64>,
    /// Whether this is a symmetric partition (both directions blocked).
    pub symmetric: bool,
}

impl Partition {
    /// Checks if this partition blocks communication from `from` to `to`.
    pub fn blocks(&self, from: u64, to: u64) -> bool {
        let a_to_b = self.group_a.contains(&from) && self.group_b.contains(&to);
        let b_to_a = self.group_b.contains(&from) && self.group_a.contains(&to);

        if self.symmetric {
            a_to_b || b_to_a
        } else {
            a_to_b // Only A -> B is blocked
        }
    }
}

// ============================================================================
// Simulated Network
// ============================================================================

/// Simulated network for deterministic testing.
///
/// Models message passing between nodes with configurable delays,
/// loss, and partitions.
#[derive(Debug)]
pub struct SimNetwork {
    /// Network configuration.
    config: NetworkConfig,
    /// Messages in transit, keyed by (from, to).
    in_flight: HashMap<(u64, u64), VecDeque<Message>>,
    /// Active partitions.
    partitions: HashMap<u64, Partition>,
    /// Known nodes in the network.
    nodes: HashSet<u64>,
    /// Counter for generating message IDs.
    next_message_id: u64,
    /// Counter for generating partition IDs.
    next_partition_id: u64,
    /// Statistics.
    stats: NetworkStats,
}

/// Network statistics for monitoring.
#[derive(Debug, Clone, Default)]
pub struct NetworkStats {
    /// Total messages sent.
    pub messages_sent: u64,
    /// Total messages delivered.
    pub messages_delivered: u64,
    /// Total messages dropped.
    pub messages_dropped: u64,
    /// Total messages duplicated.
    pub messages_duplicated: u64,
    /// Total messages rejected due to partition.
    pub messages_partitioned: u64,
}

impl SimNetwork {
    /// Creates a new simulated network with the given configuration.
    pub fn new(config: NetworkConfig) -> Self {
        Self {
            config,
            in_flight: HashMap::new(),
            partitions: HashMap::new(),
            nodes: HashSet::new(),
            next_message_id: 0,
            next_partition_id: 0,
            stats: NetworkStats::default(),
        }
    }

    /// Creates a network with default (reliable) configuration.
    pub fn reliable() -> Self {
        Self::new(NetworkConfig::reliable())
    }

    /// Registers a node in the network.
    pub fn register_node(&mut self, node_id: u64) {
        self.nodes.insert(node_id);
    }

    /// Removes a node from the network.
    pub fn unregister_node(&mut self, node_id: u64) {
        self.nodes.remove(&node_id);
        // Remove all in-flight messages to/from this node
        self.in_flight.retain(|(from, to), _| *from != node_id && *to != node_id);
    }

    /// Checks if a node is registered.
    pub fn has_node(&self, node_id: u64) -> bool {
        self.nodes.contains(&node_id)
    }

    /// Returns the number of registered nodes.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Sends a message through the network.
    ///
    /// Returns the result of the send attempt, including the delivery time
    /// if the message was queued.
    pub fn send(
        &mut self,
        from: u64,
        to: u64,
        payload: Vec<u8>,
        current_time_ns: u64,
        rng: &mut SimRng,
    ) -> SendResult {
        self.stats.messages_sent += 1;

        // Check if destination exists
        if !self.nodes.contains(&to) {
            return SendResult::Rejected {
                reason: RejectReason::UnknownDestination,
            };
        }

        // Check for partitions
        if self.is_partitioned(from, to) {
            self.stats.messages_partitioned += 1;
            return SendResult::Rejected {
                reason: RejectReason::Partitioned,
            };
        }

        // Check for queue capacity
        let queue = self.in_flight.entry((from, to)).or_default();
        if queue.len() >= self.config.max_in_flight {
            return SendResult::Rejected {
                reason: RejectReason::QueueFull,
            };
        }

        // Check for message drop
        if self.config.drop_probability > 0.0
            && rng.next_bool_with_probability(self.config.drop_probability)
        {
            self.stats.messages_dropped += 1;
            return SendResult::Dropped;
        }

        // Calculate delivery time
        let delay = rng.delay_ns(self.config.min_delay_ns, self.config.max_delay_ns);
        let deliver_at_ns = current_time_ns + delay;

        // Create message
        let message_id = MessageId(self.next_message_id);
        self.next_message_id += 1;

        let message = Message {
            id: message_id,
            from,
            to,
            payload: payload.clone(),
            sent_at_ns: current_time_ns,
            deliver_at_ns,
        };

        queue.push_back(message);

        // Handle duplication
        if self.config.duplicate_probability > 0.0
            && rng.next_bool_with_probability(self.config.duplicate_probability)
        {
            self.stats.messages_duplicated += 1;
            // Add a duplicate with a different delay
            let dup_delay = rng.delay_ns(self.config.min_delay_ns, self.config.max_delay_ns);
            let dup_deliver_at_ns = current_time_ns + dup_delay;

            let dup_id = MessageId(self.next_message_id);
            self.next_message_id += 1;

            let duplicate = Message {
                id: dup_id,
                from,
                to,
                payload,
                sent_at_ns: current_time_ns,
                deliver_at_ns: dup_deliver_at_ns,
            };
            queue.push_back(duplicate);
        }

        SendResult::Queued {
            message_id,
            deliver_at_ns,
        }
    }

    /// Delivers all messages that should be delivered by the given time.
    ///
    /// Returns the delivered messages in delivery order.
    pub fn deliver_ready(&mut self, current_time_ns: u64) -> Vec<Message> {
        let mut delivered = Vec::new();

        for queue in self.in_flight.values_mut() {
            // Collect messages ready for delivery
            let ready: Vec<_> = queue
                .iter()
                .enumerate()
                .filter(|(_, msg)| msg.deliver_at_ns <= current_time_ns)
                .map(|(idx, _)| idx)
                .collect();

            // Remove from back to front to preserve indices
            for idx in ready.into_iter().rev() {
                if let Some(msg) = queue.remove(idx) {
                    self.stats.messages_delivered += 1;
                    delivered.push(msg);
                }
            }
        }

        // Sort by delivery time for deterministic ordering
        delivered.sort_by_key(|m| (m.deliver_at_ns, m.id.0));
        delivered
    }

    /// Returns the next delivery time across all in-flight messages.
    pub fn next_delivery_time(&self) -> Option<u64> {
        self.in_flight
            .values()
            .flat_map(|q| q.iter())
            .map(|m| m.deliver_at_ns)
            .min()
    }

    /// Creates a network partition.
    ///
    /// Returns the partition ID.
    pub fn create_partition(
        &mut self,
        group_a: HashSet<u64>,
        group_b: HashSet<u64>,
        symmetric: bool,
    ) -> u64 {
        let id = self.next_partition_id;
        self.next_partition_id += 1;

        let partition = Partition {
            id,
            group_a,
            group_b,
            symmetric,
        };

        self.partitions.insert(id, partition);
        id
    }

    /// Removes a network partition (heals the network).
    pub fn heal_partition(&mut self, partition_id: u64) -> bool {
        self.partitions.remove(&partition_id).is_some()
    }

    /// Checks if communication from `from` to `to` is blocked by any partition.
    pub fn is_partitioned(&self, from: u64, to: u64) -> bool {
        self.partitions.values().any(|p| p.blocks(from, to))
    }

    /// Returns the number of active partitions.
    pub fn partition_count(&self) -> usize {
        self.partitions.len()
    }

    /// Returns the total number of in-flight messages.
    pub fn in_flight_count(&self) -> usize {
        self.in_flight.values().map(VecDeque::len).sum()
    }

    /// Returns network statistics.
    pub fn stats(&self) -> &NetworkStats {
        &self.stats
    }

    /// Clears all in-flight messages (e.g., after a node crash).
    pub fn clear_in_flight_for_node(&mut self, node_id: u64) {
        self.in_flight.retain(|(from, to), _| *from != node_id && *to != node_id);
    }

    /// Returns the configuration.
    pub fn config(&self) -> &NetworkConfig {
        &self.config
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_and_deliver_message() {
        let mut net = SimNetwork::reliable();
        let mut rng = SimRng::new(42);

        net.register_node(1);
        net.register_node(2);

        let result = net.send(1, 2, b"hello".to_vec(), 0, &mut rng);

        match result {
            SendResult::Queued { deliver_at_ns, .. } => {
                assert!(deliver_at_ns > 0);

                // Deliver at the right time
                let messages = net.deliver_ready(deliver_at_ns);
                assert_eq!(messages.len(), 1);
                assert_eq!(messages[0].payload, b"hello");
                assert_eq!(messages[0].from, 1);
                assert_eq!(messages[0].to, 2);
            }
            _ => panic!("expected Queued"),
        }
    }

    #[test]
    fn message_to_unknown_node_rejected() {
        let mut net = SimNetwork::reliable();
        let mut rng = SimRng::new(42);

        net.register_node(1);
        // Node 2 not registered

        let result = net.send(1, 2, b"hello".to_vec(), 0, &mut rng);

        assert_eq!(
            result,
            SendResult::Rejected {
                reason: RejectReason::UnknownDestination
            }
        );
    }

    #[test]
    fn partition_blocks_messages() {
        let mut net = SimNetwork::reliable();
        let mut rng = SimRng::new(42);

        net.register_node(1);
        net.register_node(2);
        net.register_node(3);

        // Partition: {1} can't reach {2, 3}
        let mut group_a = HashSet::new();
        group_a.insert(1);
        let mut group_b = HashSet::new();
        group_b.insert(2);
        group_b.insert(3);

        let partition_id = net.create_partition(group_a, group_b, true);

        // 1 -> 2 should be blocked
        let result = net.send(1, 2, b"hello".to_vec(), 0, &mut rng);
        assert_eq!(
            result,
            SendResult::Rejected {
                reason: RejectReason::Partitioned
            }
        );

        // 2 -> 3 should work (same partition group)
        let result = net.send(2, 3, b"hello".to_vec(), 0, &mut rng);
        assert!(matches!(result, SendResult::Queued { .. }));

        // Heal partition
        assert!(net.heal_partition(partition_id));

        // Now 1 -> 2 should work
        let result = net.send(1, 2, b"hello".to_vec(), 0, &mut rng);
        assert!(matches!(result, SendResult::Queued { .. }));
    }

    #[test]
    fn asymmetric_partition() {
        let mut net = SimNetwork::reliable();
        let mut rng = SimRng::new(42);

        net.register_node(1);
        net.register_node(2);

        // Asymmetric partition: 1 -> 2 blocked, but 2 -> 1 works
        let mut group_a = HashSet::new();
        group_a.insert(1);
        let mut group_b = HashSet::new();
        group_b.insert(2);

        net.create_partition(group_a, group_b, false);

        // 1 -> 2 blocked
        let result = net.send(1, 2, b"hello".to_vec(), 0, &mut rng);
        assert_eq!(
            result,
            SendResult::Rejected {
                reason: RejectReason::Partitioned
            }
        );

        // 2 -> 1 works
        let result = net.send(2, 1, b"hello".to_vec(), 0, &mut rng);
        assert!(matches!(result, SendResult::Queued { .. }));
    }

    #[test]
    fn message_dropping() {
        let config = NetworkConfig {
            drop_probability: 1.0, // Always drop
            ..NetworkConfig::default()
        };
        let mut net = SimNetwork::new(config);
        let mut rng = SimRng::new(42);

        net.register_node(1);
        net.register_node(2);

        let result = net.send(1, 2, b"hello".to_vec(), 0, &mut rng);
        assert_eq!(result, SendResult::Dropped);
        assert_eq!(net.stats().messages_dropped, 1);
    }

    #[test]
    fn multiple_messages_delivered_in_order() {
        let mut net = SimNetwork::reliable();
        let mut rng = SimRng::new(42);

        net.register_node(1);
        net.register_node(2);

        // Send multiple messages
        for i in 0..5 {
            net.send(1, 2, vec![i], 0, &mut rng);
        }

        // Get delivery time for all
        let deliver_time = net.next_delivery_time().unwrap() + 1_000_000_000; // Far future

        let messages = net.deliver_ready(deliver_time);
        assert_eq!(messages.len(), 5);

        // Messages should be in deterministic order
        // (sorted by deliver_at_ns, then by message_id)
    }

    #[test]
    fn stats_tracking() {
        let mut net = SimNetwork::reliable();
        let mut rng = SimRng::new(42);

        net.register_node(1);
        net.register_node(2);

        net.send(1, 2, b"msg1".to_vec(), 0, &mut rng);
        net.send(1, 2, b"msg2".to_vec(), 0, &mut rng);

        assert_eq!(net.stats().messages_sent, 2);
        assert_eq!(net.stats().messages_delivered, 0);

        let deliver_time = net.next_delivery_time().unwrap() + 1_000_000_000;
        net.deliver_ready(deliver_time);

        assert_eq!(net.stats().messages_delivered, 2);
    }

    #[test]
    fn queue_capacity_limit() {
        let config = NetworkConfig {
            max_in_flight: 2,
            ..NetworkConfig::default()
        };
        let mut net = SimNetwork::new(config);
        let mut rng = SimRng::new(42);

        net.register_node(1);
        net.register_node(2);

        // Fill the queue
        assert!(matches!(
            net.send(1, 2, b"msg1".to_vec(), 0, &mut rng),
            SendResult::Queued { .. }
        ));
        assert!(matches!(
            net.send(1, 2, b"msg2".to_vec(), 0, &mut rng),
            SendResult::Queued { .. }
        ));

        // Queue full
        assert_eq!(
            net.send(1, 2, b"msg3".to_vec(), 0, &mut rng),
            SendResult::Rejected {
                reason: RejectReason::QueueFull
            }
        );
    }
}
