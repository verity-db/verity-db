//! Event loop driver for VSR multi-node replication.
//!
//! This module provides a mio-based event loop that drives the `ReplicaState`
//! state machine with network events and timeouts.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                         EventLoop                                │
//! ├─────────────────────────────────────────────────────────────────┤
//! │  1. Poll transport for messages                                  │
//! │  2. Check timeouts (heartbeat, prepare, view change)            │
//! │  3. Process pending client requests                              │
//! │  4. Drive ReplicaState::process() for each event                │
//! │  5. Send outgoing messages via transport                        │
//! │  6. Update superblock on state changes                          │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Threading Model
//!
//! The event loop runs in a dedicated thread, separate from client I/O.
//! Communication with the main server thread is via channels:
//! - `command_tx`: Send commands to the replicator
//! - `result_rx`: Receive results from committed commands

use std::collections::HashMap;
use std::io::{Read, Seek, Write};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use tracing::{debug, info, trace, warn};
use craton_kernel::{Command, Effect};
use craton_types::IdempotencyId;

use crate::VsrError;
use crate::config::{ClusterConfig, TimeoutConfig};
use crate::replica::{ReplicaEvent, ReplicaOutput, ReplicaState, TimeoutKind};
use crate::superblock::Superblock;
use crate::tcp_transport::TcpTransport;
use crate::transport::Transport;
use crate::types::{CommitNumber, OpNumber, ReplicaId, ReplicaStatus, ViewNumber};

// ============================================================================
// Event Loop Configuration
// ============================================================================

/// Configuration for the event loop.
#[derive(Debug, Clone)]
pub struct EventLoopConfig {
    /// Timeout configuration.
    pub timeouts: TimeoutConfig,
    /// How often to poll the transport (microseconds).
    pub poll_interval: Duration,
    /// Maximum commands to process per iteration.
    pub max_commands_per_tick: usize,
}

impl Default for EventLoopConfig {
    fn default() -> Self {
        Self {
            timeouts: TimeoutConfig::development(),
            poll_interval: Duration::from_millis(1),
            max_commands_per_tick: 100,
        }
    }
}

impl EventLoopConfig {
    /// Creates a configuration for production use.
    pub fn production() -> Self {
        Self {
            timeouts: TimeoutConfig::production(),
            poll_interval: Duration::from_millis(1),
            max_commands_per_tick: 1000,
        }
    }

    /// Creates a configuration for development/testing.
    pub fn development() -> Self {
        Self::default()
    }
}

// ============================================================================
// Commands and Results
// ============================================================================

/// Commands that can be sent to the event loop.
#[derive(Debug)]
pub enum EventLoopCommand {
    /// Submit a client request.
    Submit {
        command: Command,
        idempotency_id: Option<IdempotencyId>,
        result_tx: SyncSender<Result<SubmitResponse, VsrError>>,
    },
    /// Shutdown the event loop.
    Shutdown,
}

/// Response from a successful command submission.
#[derive(Debug)]
pub struct SubmitResponse {
    /// The operation number assigned.
    pub op_number: OpNumber,
    /// Effects produced by the kernel.
    pub effects: Vec<Effect>,
    /// Whether this was a duplicate (idempotency hit).
    pub was_duplicate: bool,
}

// ============================================================================
// Shared State
// ============================================================================

/// State shared between the event loop and external callers.
#[derive(Debug)]
pub struct SharedState {
    /// Current view number.
    pub view: ViewNumber,
    /// Current commit number.
    pub commit_number: CommitNumber,
    /// Current replica status.
    pub status: ReplicaStatus,
    /// Whether this replica is the leader.
    pub is_leader: bool,
    /// Number of connected peers.
    pub connected_peers: usize,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            view: ViewNumber::ZERO,
            commit_number: CommitNumber::ZERO,
            status: ReplicaStatus::Normal,
            is_leader: false,
            connected_peers: 0,
        }
    }
}

// ============================================================================
// Timeout Tracker
// ============================================================================

/// Tracks and manages protocol timeouts.
struct TimeoutTracker {
    config: TimeoutConfig,
    /// Last time we received a heartbeat from the leader.
    last_heartbeat: Instant,
    /// Pending prepare timeouts: `op_number` -> deadline.
    prepare_deadlines: HashMap<OpNumber, Instant>,
    /// View change deadline (if in view change).
    view_change_deadline: Option<Instant>,
    /// Recovery deadline (if recovering).
    recovery_deadline: Option<Instant>,
}

impl TimeoutTracker {
    fn new(config: TimeoutConfig) -> Self {
        Self {
            config,
            last_heartbeat: Instant::now(),
            prepare_deadlines: HashMap::new(),
            view_change_deadline: None,
            recovery_deadline: None,
        }
    }

    /// Resets the heartbeat timer.
    fn reset_heartbeat(&mut self) {
        self.last_heartbeat = Instant::now();
    }

    /// Starts tracking a prepare timeout.
    fn start_prepare(&mut self, op: OpNumber) {
        let deadline = Instant::now() + self.config.prepare_timeout;
        self.prepare_deadlines.insert(op, deadline);
    }

    /// Cancels a prepare timeout.
    fn cancel_prepare(&mut self, op: OpNumber) {
        self.prepare_deadlines.remove(&op);
    }

    /// Starts view change timeout.
    #[allow(dead_code)]
    fn start_view_change(&mut self) {
        self.view_change_deadline = Some(Instant::now() + self.config.view_change_timeout);
    }

    /// Cancels view change timeout.
    #[allow(dead_code)]
    fn cancel_view_change(&mut self) {
        self.view_change_deadline = None;
    }

    /// Starts recovery timeout.
    #[allow(dead_code)]
    fn start_recovery(&mut self) {
        self.recovery_deadline = Some(Instant::now() + self.config.recovery_retry);
    }

    /// Cancels recovery timeout.
    #[allow(dead_code)]
    fn cancel_recovery(&mut self) {
        self.recovery_deadline = None;
    }

    /// Returns fired timeouts.
    fn check_timeouts(&mut self, is_leader: bool) -> Vec<TimeoutKind> {
        let now = Instant::now();
        let mut fired = Vec::new();

        // Heartbeat timeout (only for non-leaders)
        // Use view_change_timeout as the threshold for missing heartbeats
        if !is_leader && now.duration_since(self.last_heartbeat) > self.config.view_change_timeout {
            fired.push(TimeoutKind::Heartbeat);
            self.reset_heartbeat(); // Prevent repeated firing
        }

        // Prepare timeouts
        let expired_prepares: Vec<_> = self
            .prepare_deadlines
            .iter()
            .filter(|(_, deadline)| now > **deadline)
            .map(|(op, _)| *op)
            .collect();

        for op in expired_prepares {
            self.prepare_deadlines.remove(&op);
            fired.push(TimeoutKind::Prepare(op));
        }

        // View change timeout
        if let Some(deadline) = self.view_change_deadline {
            if now > deadline {
                self.view_change_deadline = None;
                fired.push(TimeoutKind::ViewChange);
            }
        }

        // Recovery timeout
        if let Some(deadline) = self.recovery_deadline {
            if now > deadline {
                self.recovery_deadline = None;
                fired.push(TimeoutKind::Recovery);
            }
        }

        fired
    }
}

// ============================================================================
// Event Loop Handle
// ============================================================================

/// Handle for interacting with the event loop from outside.
#[derive(Clone)]
pub struct EventLoopHandle {
    /// Channel for sending commands.
    command_tx: SyncSender<EventLoopCommand>,
    /// Shared state (for status queries).
    shared_state: Arc<RwLock<SharedState>>,
}

impl EventLoopHandle {
    /// Submits a command and waits for the result.
    pub fn submit(
        &self,
        command: Command,
        idempotency_id: Option<IdempotencyId>,
    ) -> Result<SubmitResponse, VsrError> {
        let (result_tx, result_rx) = mpsc::sync_channel(1);

        self.command_tx
            .send(EventLoopCommand::Submit {
                command,
                idempotency_id,
                result_tx,
            })
            .map_err(|_| VsrError::InvalidState {
                status: ReplicaStatus::Normal,
                expected: "event loop running",
            })?;

        result_rx.recv().map_err(|_| VsrError::InvalidState {
            status: ReplicaStatus::Normal,
            expected: "event loop response",
        })?
    }

    /// Returns the current shared state.
    pub fn state(&self) -> SharedState {
        self.shared_state
            .read()
            .map(|s| SharedState {
                view: s.view,
                commit_number: s.commit_number,
                status: s.status,
                is_leader: s.is_leader,
                connected_peers: s.connected_peers,
            })
            .unwrap_or_default()
    }

    /// Requests shutdown of the event loop.
    pub fn shutdown(&self) {
        let _ = self.command_tx.send(EventLoopCommand::Shutdown);
    }

    /// Returns true if this replica is the leader.
    pub fn is_leader(&self) -> bool {
        self.shared_state
            .read()
            .map(|s| s.is_leader)
            .unwrap_or(false)
    }

    /// Returns the current view.
    pub fn view(&self) -> ViewNumber {
        self.shared_state
            .read()
            .map(|s| s.view)
            .unwrap_or(ViewNumber::ZERO)
    }

    /// Returns the current commit number.
    pub fn commit_number(&self) -> CommitNumber {
        self.shared_state
            .read()
            .map(|s| s.commit_number)
            .unwrap_or(CommitNumber::ZERO)
    }
}

impl std::fmt::Debug for EventLoopHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventLoopHandle")
            .field("state", &self.state())
            .finish()
    }
}

// ============================================================================
// Event Loop
// ============================================================================

/// The main event loop driver for VSR replication.
///
/// This struct contains the state needed to run the event loop. It should be
/// run in a dedicated thread via `EventLoop::run()`.
pub struct EventLoop<S> {
    /// Configuration.
    config: EventLoopConfig,
    /// Cluster configuration.
    cluster_config: ClusterConfig,
    /// The replica state machine.
    replica_state: ReplicaState,
    /// Transport for network communication.
    transport: TcpTransport,
    /// Superblock for persistent state.
    superblock: Superblock<S>,
    /// Timeout tracker.
    timeouts: TimeoutTracker,
    /// Channel for receiving commands.
    command_rx: Receiver<EventLoopCommand>,
    /// Shared state.
    shared_state: Arc<RwLock<SharedState>>,
    /// Pending client requests waiting for commit.
    pending_commits: HashMap<OpNumber, SyncSender<Result<SubmitResponse, VsrError>>>,
    /// Time of last tick.
    last_tick: Instant,
    /// Heartbeat interval.
    heartbeat_interval: Duration,
    /// Time of last heartbeat sent (for leader).
    last_heartbeat_sent: Instant,
    /// Running flag.
    running: bool,
}

impl<S: Read + Write + Seek> EventLoop<S> {
    /// Creates a new event loop.
    pub fn new(
        config: EventLoopConfig,
        cluster_config: ClusterConfig,
        replica_id: ReplicaId,
        transport: TcpTransport,
        superblock: Superblock<S>,
    ) -> (Self, EventLoopHandle) {
        let (command_tx, command_rx) = mpsc::sync_channel(1000);
        let shared_state = Arc::new(RwLock::new(SharedState::default()));

        let replica_state = ReplicaState::new(replica_id, cluster_config.clone());
        let timeouts = TimeoutTracker::new(config.timeouts);

        let event_loop = Self {
            heartbeat_interval: config.timeouts.heartbeat_interval / 2,
            config,
            cluster_config,
            replica_state,
            transport,
            superblock,
            timeouts,
            command_rx,
            shared_state: Arc::clone(&shared_state),
            pending_commits: HashMap::new(),
            last_tick: Instant::now(),
            last_heartbeat_sent: Instant::now(),
            running: true,
        };

        let handle = EventLoopHandle {
            command_tx,
            shared_state,
        };

        (event_loop, handle)
    }

    /// Runs the event loop until shutdown.
    ///
    /// This method blocks and should be run in a dedicated thread.
    pub fn run(&mut self) -> Result<(), VsrError> {
        info!(
            replica = %self.replica_state.replica_id(),
            "event loop starting"
        );

        // Initial state update
        self.update_shared_state();

        while self.running {
            // 1. Poll transport for messages
            let messages = self
                .transport
                .poll(Some(self.config.poll_interval))
                .map_err(VsrError::Storage)?;

            // 2. Process received messages
            for msg in messages {
                self.process_event(ReplicaEvent::Message(msg))?;
            }

            // 3. Check and process timeouts
            let fired_timeouts = self.timeouts.check_timeouts(self.replica_state.is_leader());
            for timeout in fired_timeouts {
                self.process_event(ReplicaEvent::Timeout(timeout))?;
            }

            // 4. Process pending commands
            self.process_commands()?;

            // 5. Leader: send periodic heartbeats
            if self.replica_state.is_leader()
                && self.replica_state.status() == ReplicaStatus::Normal
            {
                let now = Instant::now();
                if now.duration_since(self.last_heartbeat_sent) > self.heartbeat_interval {
                    self.process_event(ReplicaEvent::Tick)?;
                    self.last_heartbeat_sent = now;
                }
            }

            // 6. Periodic tick for housekeeping
            let now = Instant::now();
            if now.duration_since(self.last_tick) > Duration::from_millis(100) {
                self.last_tick = now;
                // Could add additional periodic tasks here
            }
        }

        info!(
            replica = %self.replica_state.replica_id(),
            "event loop stopped"
        );

        Ok(())
    }

    /// Processes a single event.
    fn process_event(&mut self, event: ReplicaEvent) -> Result<(), VsrError> {
        trace!(event = ?event, "processing event");

        // Take ownership for pure state transition
        let state = std::mem::replace(
            &mut self.replica_state,
            ReplicaState::new(ReplicaId::new(0), self.cluster_config.clone()),
        );

        let (new_state, output) = state.process(event);
        self.replica_state = new_state;

        // Handle output
        self.handle_output(output)?;

        // Update shared state
        self.update_shared_state();

        Ok(())
    }

    /// Handles output from the replica state machine.
    fn handle_output(&mut self, output: ReplicaOutput) -> Result<(), VsrError> {
        // Send messages
        for msg in output.messages {
            if msg.is_broadcast() {
                // Broadcast to all peers
                let others: Vec<_> = self
                    .cluster_config
                    .others(self.replica_state.replica_id())
                    .collect();
                for peer in others {
                    self.transport.send(peer, msg.clone());
                }
            } else if let Some(to) = msg.to {
                self.transport.send(to, msg);
            }
        }

        // Handle committed operation
        if let Some(op) = output.committed_op {
            debug!(op = %op, "operation committed");

            // Notify pending client if any
            if let Some(result_tx) = self.pending_commits.remove(&op) {
                let response = SubmitResponse {
                    op_number: op,
                    effects: output.effects, // Take ownership
                    was_duplicate: false,
                };
                let _ = result_tx.send(Ok(response));
            }

            // Update superblock
            self.superblock.update(
                self.replica_state.view(),
                self.replica_state.op_number(),
                self.replica_state.commit_number(),
            )?;

            // Cancel prepare timeout
            self.timeouts.cancel_prepare(op);
        }

        Ok(())
    }

    /// Processes pending commands from the channel.
    fn process_commands(&mut self) -> Result<(), VsrError> {
        let mut processed = 0;

        while processed < self.config.max_commands_per_tick {
            match self.command_rx.try_recv() {
                Ok(EventLoopCommand::Submit {
                    command,
                    idempotency_id,
                    result_tx,
                }) => {
                    processed += 1;

                    // Check if we can accept requests
                    if !self.replica_state.can_accept_requests() {
                        let _ = result_tx.send(Err(VsrError::NotLeader {
                            view: self.replica_state.view(),
                        }));
                        continue;
                    }

                    // Process the client request
                    let event = ReplicaEvent::ClientRequest {
                        command,
                        idempotency_id,
                    };

                    // Get the next op number before processing
                    let expected_op = self.replica_state.op_number().next();

                    self.process_event(event)?;

                    // Track pending commit
                    self.pending_commits.insert(expected_op, result_tx);

                    // Start prepare timeout
                    self.timeouts.start_prepare(expected_op);
                }
                Ok(EventLoopCommand::Shutdown) => {
                    info!("shutdown requested");
                    self.running = false;
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    warn!("command channel disconnected");
                    self.running = false;
                    break;
                }
            }
        }

        Ok(())
    }

    /// Updates the shared state from the replica state.
    fn update_shared_state(&self) {
        if let Ok(mut state) = self.shared_state.write() {
            state.view = self.replica_state.view();
            state.commit_number = self.replica_state.commit_number();
            state.status = self.replica_state.status();
            state.is_leader = self.replica_state.is_leader();
            state.connected_peers = self.transport.connected_count();
        }
    }

    /// Called when receiving a message from the leader (resets heartbeat timeout).
    #[allow(dead_code)]
    fn on_leader_message(&mut self) {
        self.timeouts.reset_heartbeat();
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_loop_config_defaults() {
        let config = EventLoopConfig::default();
        assert_eq!(config.poll_interval, Duration::from_millis(1));
        assert_eq!(config.max_commands_per_tick, 100);
    }

    #[test]
    fn shared_state_default() {
        let state = SharedState::default();
        assert_eq!(state.view, ViewNumber::ZERO);
        assert_eq!(state.commit_number, CommitNumber::ZERO);
        assert_eq!(state.status, ReplicaStatus::Normal);
        assert!(!state.is_leader);
    }

    #[test]
    fn timeout_tracker_heartbeat() {
        let config = TimeoutConfig::development();
        let mut tracker = TimeoutTracker::new(config);

        // No timeout initially
        assert!(tracker.check_timeouts(false).is_empty());

        // Simulate time passing
        tracker.last_heartbeat = Instant::now() - Duration::from_secs(10);

        // Should fire heartbeat timeout (not leader)
        let timeouts = tracker.check_timeouts(false);
        assert!(timeouts.contains(&TimeoutKind::Heartbeat));

        // Should not fire for leader
        tracker.last_heartbeat = Instant::now() - Duration::from_secs(10);
        let timeouts = tracker.check_timeouts(true);
        assert!(!timeouts.contains(&TimeoutKind::Heartbeat));
    }

    #[test]
    fn timeout_tracker_prepare() {
        let config = TimeoutConfig {
            prepare_timeout: Duration::from_millis(10),
            ..TimeoutConfig::development()
        };
        let mut tracker = TimeoutTracker::new(config);

        tracker.start_prepare(OpNumber::new(1));

        // Not expired yet
        assert!(tracker.check_timeouts(true).is_empty());

        // Simulate expiry
        tracker
            .prepare_deadlines
            .insert(OpNumber::new(1), Instant::now() - Duration::from_millis(1));

        let timeouts = tracker.check_timeouts(true);
        assert!(timeouts.contains(&TimeoutKind::Prepare(OpNumber::new(1))));
    }
}
