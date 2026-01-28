//! Event scheduling and priority queue for discrete event simulation.
//!
//! The event system is the heart of the simulation harness. Events are
//! scheduled at specific simulation times and processed in order.
//!
//! # Design
//!
//! Events are processed in strict time order. When multiple events are
//! scheduled for the same time, they are processed in FIFO order (by
//! insertion order).

use std::cmp::Ordering;
use std::collections::BinaryHeap;

// ============================================================================
// Event Types
// ============================================================================

/// Unique identifier for an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EventId(u64);

impl EventId {
    /// Creates an event ID from a raw value.
    pub fn from_raw(id: u64) -> Self {
        Self(id)
    }

    /// Returns the raw event ID.
    pub fn as_raw(&self) -> u64 {
        self.0
    }
}

/// The kind of event being scheduled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventKind {
    /// A custom event with an application-defined type code.
    Custom(u64),

    /// A timer event (e.g., heartbeat, timeout).
    Timer { timer_id: u64 },

    /// A network message delivery event.
    NetworkDeliver { from: u64, to: u64, message_id: u64 },

    /// A storage I/O completion event.
    StorageComplete { operation_id: u64, success: bool },

    /// A node crash event.
    NodeCrash { node_id: u64 },

    /// A node restart event.
    NodeRestart { node_id: u64 },

    /// A network partition event.
    NetworkPartition { partition_id: u64 },

    /// A network heal event (partition removed).
    NetworkHeal { partition_id: u64 },

    /// An invariant check event.
    InvariantCheck,
}

/// A scheduled event in the simulation.
#[derive(Debug, Clone)]
pub struct Event {
    /// The event ID (unique within the simulation).
    pub id: EventId,
    /// The time at which the event is scheduled (nanoseconds).
    pub time_ns: u64,
    /// The kind of event.
    pub kind: EventKind,
    /// Insertion order for tie-breaking (lower = earlier).
    insertion_order: u64,
}

// ============================================================================
// Event Queue
// ============================================================================

/// Wrapper for events in the priority queue.
///
/// Implements ordering so that:
/// 1. Events with lower time come first
/// 2. Events at the same time are ordered by insertion order (FIFO)
#[derive(Debug)]
struct QueuedEvent(Event);

impl PartialEq for QueuedEvent {
    fn eq(&self, other: &Self) -> bool {
        self.0.time_ns == other.0.time_ns && self.0.insertion_order == other.0.insertion_order
    }
}

impl Eq for QueuedEvent {}

impl PartialOrd for QueuedEvent {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for QueuedEvent {
    fn cmp(&self, other: &Self) -> Ordering {
        // BinaryHeap is a max-heap, so we reverse the comparison
        // to get a min-heap (earliest events first)
        match other.0.time_ns.cmp(&self.0.time_ns) {
            Ordering::Equal => other.0.insertion_order.cmp(&self.0.insertion_order),
            other_ordering => other_ordering,
        }
    }
}

/// Priority queue of scheduled events.
///
/// Events are processed in time order. When multiple events are scheduled
/// for the same time, they are processed in insertion order (FIFO).
#[derive(Debug)]
pub struct EventQueue {
    /// The priority queue of events.
    heap: BinaryHeap<QueuedEvent>,
    /// Counter for generating unique event IDs.
    next_id: u64,
    /// Counter for insertion ordering (tie-breaking).
    insertion_counter: u64,
}

impl EventQueue {
    /// Creates a new empty event queue.
    pub fn new() -> Self {
        Self {
            heap: BinaryHeap::new(),
            next_id: 0,
            insertion_counter: 0,
        }
    }

    /// Creates a queue with a pre-allocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            heap: BinaryHeap::with_capacity(capacity),
            next_id: 0,
            insertion_counter: 0,
        }
    }

    /// Schedules an event at the specified time.
    ///
    /// Returns the event ID for the scheduled event.
    pub fn schedule(&mut self, time_ns: u64, kind: EventKind) -> EventId {
        let id = EventId(self.next_id);
        self.next_id += 1;

        let event = Event {
            id,
            time_ns,
            kind,
            insertion_order: self.insertion_counter,
        };
        self.insertion_counter += 1;

        self.heap.push(QueuedEvent(event));
        id
    }

    /// Removes and returns the next event to process.
    ///
    /// Returns `None` if the queue is empty.
    pub fn pop(&mut self) -> Option<Event> {
        self.heap.pop().map(|qe| qe.0)
    }

    /// Peeks at the next event without removing it.
    pub fn peek(&self) -> Option<&Event> {
        self.heap.peek().map(|qe| &qe.0)
    }

    /// Returns the number of pending events.
    pub fn len(&self) -> usize {
        self.heap.len()
    }

    /// Returns true if there are no pending events.
    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }

    /// Returns the time of the next event, if any.
    pub fn next_time(&self) -> Option<u64> {
        self.peek().map(|e| e.time_ns)
    }

    /// Clears all pending events.
    pub fn clear(&mut self) {
        self.heap.clear();
    }
}

impl Default for EventQueue {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_come_out_in_time_order() {
        let mut queue = EventQueue::new();

        queue.schedule(3_000_000, EventKind::Custom(3));
        queue.schedule(1_000_000, EventKind::Custom(1));
        queue.schedule(2_000_000, EventKind::Custom(2));

        let e1 = queue.pop().unwrap();
        let e2 = queue.pop().unwrap();
        let e3 = queue.pop().unwrap();

        assert_eq!(e1.time_ns, 1_000_000);
        assert_eq!(e2.time_ns, 2_000_000);
        assert_eq!(e3.time_ns, 3_000_000);

        assert!(queue.pop().is_none());
    }

    #[test]
    fn same_time_events_are_fifo() {
        let mut queue = EventQueue::new();

        // Schedule 3 events at the same time
        queue.schedule(1_000_000, EventKind::Custom(1));
        queue.schedule(1_000_000, EventKind::Custom(2));
        queue.schedule(1_000_000, EventKind::Custom(3));

        // They should come out in insertion order
        let e1 = queue.pop().unwrap();
        let e2 = queue.pop().unwrap();
        let e3 = queue.pop().unwrap();

        // Extract the custom values to check order
        match (&e1.kind, &e2.kind, &e3.kind) {
            (EventKind::Custom(1), EventKind::Custom(2), EventKind::Custom(3)) => {}
            _ => panic!("events not in FIFO order"),
        }
    }

    #[test]
    fn peek_doesnt_remove() {
        let mut queue = EventQueue::new();
        queue.schedule(1_000_000, EventKind::Custom(1));

        assert_eq!(queue.len(), 1);
        let peeked = queue.peek().unwrap();
        assert_eq!(peeked.time_ns, 1_000_000);
        assert_eq!(queue.len(), 1); // Still there
    }

    #[test]
    fn event_ids_are_unique() {
        let mut queue = EventQueue::new();

        let id1 = queue.schedule(1_000, EventKind::Custom(1));
        let id2 = queue.schedule(2_000, EventKind::Custom(2));
        let id3 = queue.schedule(3_000, EventKind::Custom(3));

        assert_ne!(id1, id2);
        assert_ne!(id2, id3);
        assert_ne!(id1, id3);
    }

    #[test]
    fn clear_removes_all_events() {
        let mut queue = EventQueue::new();

        queue.schedule(1_000, EventKind::Custom(1));
        queue.schedule(2_000, EventKind::Custom(2));

        assert_eq!(queue.len(), 2);
        queue.clear();
        assert!(queue.is_empty());
    }

    #[test]
    fn next_time_returns_earliest() {
        let mut queue = EventQueue::new();

        queue.schedule(5_000, EventKind::Custom(1));
        queue.schedule(1_000, EventKind::Custom(2));
        queue.schedule(3_000, EventKind::Custom(3));

        assert_eq!(queue.next_time(), Some(1_000));
    }

    #[test]
    fn empty_queue() {
        let queue = EventQueue::new();

        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);
        assert!(queue.peek().is_none());
        assert!(queue.next_time().is_none());
    }
}
