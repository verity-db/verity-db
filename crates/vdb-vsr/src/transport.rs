//! Transport abstraction for VSR message passing.
//!
//! This module defines the [`Transport`] trait that abstracts over different
//! message delivery mechanisms:
//!
//! - [`NullTransport`]: For single-node operation (no network)
//! - Future: `SimTransport` for simulation testing
//! - Future: `TcpTransport` for production deployments
//!
//! # Design
//!
//! The transport layer is intentionally simple:
//! - Fire-and-forget message sending (no delivery guarantees)
//! - Messages may be lost, reordered, or duplicated
//! - The VSR protocol handles all reliability concerns
//!
//! This design enables comprehensive fault injection during simulation.

use std::fmt::Debug;

use crate::message::Message;
use crate::types::ReplicaId;

// ============================================================================
// Transport Trait
// ============================================================================

/// Abstraction for sending and receiving VSR protocol messages.
///
/// Implementations provide the actual message delivery mechanism.
/// The transport makes no guarantees about delivery - the VSR protocol
/// handles reliability through retransmission and quorum requirements.
///
/// # FCIS Pattern
///
/// Transport is part of the imperative shell. The pure replica state
/// machine produces messages as output; the transport sends them.
pub trait Transport: Debug + Send + Sync {
    /// Sends a message to a specific replica.
    ///
    /// This is a fire-and-forget operation. The message may be:
    /// - Delivered successfully
    /// - Lost in transit
    /// - Delivered out of order
    /// - Delivered multiple times
    ///
    /// The VSR protocol handles all these cases.
    fn send(&self, to: ReplicaId, message: Message);

    /// Broadcasts a message to all replicas except the sender.
    ///
    /// Default implementation calls `send` for each target.
    fn broadcast(&self, from: ReplicaId, targets: &[ReplicaId], message: Message) {
        for &target in targets {
            if target != from {
                self.send(target, message.clone());
            }
        }
    }

    /// Returns the local replica ID.
    fn local_id(&self) -> ReplicaId;
}

// ============================================================================
// Null Transport (for single-node operation)
// ============================================================================

/// A no-op transport for single-node operation.
///
/// Since single-node VSR has no peers, all send operations are no-ops.
/// This transport is used by [`SingleNodeReplicator`](crate::SingleNodeReplicator).
#[derive(Debug, Clone)]
pub struct NullTransport {
    local_id: ReplicaId,
}

impl NullTransport {
    /// Creates a new null transport for the given replica.
    pub fn new(local_id: ReplicaId) -> Self {
        Self { local_id }
    }
}

impl Transport for NullTransport {
    fn send(&self, _to: ReplicaId, _message: Message) {
        // No-op: single-node has no peers
    }

    fn broadcast(&self, _from: ReplicaId, _targets: &[ReplicaId], _message: Message) {
        // No-op: single-node has no peers
    }

    fn local_id(&self) -> ReplicaId {
        self.local_id
    }
}

// ============================================================================
// Message Sink (for testing)
// ============================================================================

/// A transport that collects messages for testing.
///
/// All sent messages are stored in a vector for later inspection.
#[derive(Debug)]
pub struct MessageSink {
    local_id: ReplicaId,
    messages: std::sync::Mutex<Vec<(ReplicaId, Message)>>,
}

impl MessageSink {
    /// Creates a new message sink.
    pub fn new(local_id: ReplicaId) -> Self {
        Self {
            local_id,
            messages: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Returns all collected messages.
    pub fn drain(&self) -> Vec<(ReplicaId, Message)> {
        let mut messages = self.messages.lock().expect("lock poisoned");
        std::mem::take(&mut *messages)
    }

    /// Returns the number of collected messages.
    pub fn len(&self) -> usize {
        self.messages.lock().expect("lock poisoned").len()
    }

    /// Returns true if no messages have been collected.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Transport for MessageSink {
    fn send(&self, to: ReplicaId, message: Message) {
        let mut messages = self.messages.lock().expect("lock poisoned");
        messages.push((to, message));
    }

    fn local_id(&self) -> ReplicaId {
        self.local_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CommitNumber, Heartbeat, MessagePayload, ViewNumber};

    fn test_message() -> Message {
        Message::broadcast(
            ReplicaId::new(0),
            MessagePayload::Heartbeat(Heartbeat::new(ViewNumber::ZERO, CommitNumber::ZERO)),
        )
    }

    #[test]
    fn null_transport_is_noop() {
        let transport = NullTransport::new(ReplicaId::new(0));

        // Should not panic
        transport.send(ReplicaId::new(1), test_message());
        transport.broadcast(
            ReplicaId::new(0),
            &[ReplicaId::new(1), ReplicaId::new(2)],
            test_message(),
        );

        assert_eq!(transport.local_id(), ReplicaId::new(0));
    }

    #[test]
    fn message_sink_collects_messages() {
        let sink = MessageSink::new(ReplicaId::new(0));

        sink.send(ReplicaId::new(1), test_message());
        sink.send(ReplicaId::new(2), test_message());

        assert_eq!(sink.len(), 2);

        let messages = sink.drain();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].0, ReplicaId::new(1));
        assert_eq!(messages[1].0, ReplicaId::new(2));

        assert!(sink.is_empty());
    }

    #[test]
    fn message_sink_broadcast() {
        let sink = MessageSink::new(ReplicaId::new(0));
        let targets = vec![ReplicaId::new(0), ReplicaId::new(1), ReplicaId::new(2)];

        sink.broadcast(ReplicaId::new(0), &targets, test_message());

        // Should send to 1 and 2, but not 0 (self)
        let messages = sink.drain();
        assert_eq!(messages.len(), 2);
        assert!(messages.iter().all(|(id, _)| *id != ReplicaId::new(0)));
    }
}
