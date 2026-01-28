//! TCP server implementation using mio for non-blocking I/O.

use std::collections::HashMap;
use std::net::SocketAddr;

use mio::net::TcpListener;
use mio::{Events, Interest, Poll, Token};
use tracing::{debug, error, info, trace, warn};
use vdb::Verity;

use crate::config::ServerConfig;
use crate::connection::Connection;
use crate::error::{ServerError, ServerResult};
use crate::handler::RequestHandler;

/// Token for the listener socket.
const LISTENER_TOKEN: Token = Token(0);

/// Maximum events to process per poll iteration.
const MAX_EVENTS: usize = 1024;

/// TCP server for `VerityDB`.
///
/// Uses mio's poll-based event loop for handling multiple connections
/// without async runtimes.
pub struct Server {
    config: ServerConfig,
    poll: Poll,
    listener: TcpListener,
    connections: HashMap<Token, Connection>,
    handler: RequestHandler,
    next_token: usize,
}

impl Server {
    /// Creates a new server with the given configuration.
    pub fn new(config: ServerConfig, db: Verity) -> ServerResult<Self> {
        let poll = Poll::new()?;

        // Bind the listener
        let addr = config.bind_addr;
        let mut listener =
            TcpListener::bind(addr).map_err(|e| ServerError::BindFailed { addr, source: e })?;

        // Register the listener with the poll
        poll.registry()
            .register(&mut listener, LISTENER_TOKEN, Interest::READABLE)?;

        info!("Server listening on {}", addr);

        Ok(Self {
            config,
            poll,
            listener,
            connections: HashMap::new(),
            handler: RequestHandler::new(db),
            next_token: 1, // Start at 1 since 0 is LISTENER_TOKEN
        })
    }

    /// Returns the address the server is listening on.
    pub fn local_addr(&self) -> ServerResult<SocketAddr> {
        Ok(self.listener.local_addr()?)
    }

    /// Runs the server event loop.
    ///
    /// This method blocks until the server is shut down.
    pub fn run(&mut self) -> ServerResult<()> {
        let mut events = Events::with_capacity(MAX_EVENTS);

        info!("Server event loop started");

        loop {
            // Wait for events
            if let Err(e) = self.poll.poll(&mut events, None) {
                if e.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e.into());
            }

            // Process events
            for event in &events {
                match event.token() {
                    LISTENER_TOKEN => {
                        self.accept_connections()?;
                    }
                    token => {
                        if event.is_readable() {
                            self.handle_readable(token)?;
                        }
                        if event.is_writable() {
                            self.handle_writable(token)?;
                        }
                    }
                }
            }

            // Clean up closed connections
            self.cleanup_closed();
        }
    }

    /// Runs a single iteration of the event loop.
    ///
    /// Useful for testing or custom event loops.
    pub fn poll_once(&mut self, timeout: Option<std::time::Duration>) -> ServerResult<()> {
        let mut events = Events::with_capacity(MAX_EVENTS);

        self.poll.poll(&mut events, timeout)?;

        for event in &events {
            match event.token() {
                LISTENER_TOKEN => {
                    self.accept_connections()?;
                }
                token => {
                    if event.is_readable() {
                        self.handle_readable(token)?;
                    }
                    if event.is_writable() {
                        self.handle_writable(token)?;
                    }
                }
            }
        }

        self.cleanup_closed();
        Ok(())
    }

    /// Accepts new connections from the listener.
    fn accept_connections(&mut self) -> ServerResult<()> {
        loop {
            match self.listener.accept() {
                Ok((mut stream, addr)) => {
                    // Check connection limit
                    if self.connections.len() >= self.config.max_connections {
                        warn!(
                            "Max connections reached, rejecting connection from {}",
                            addr
                        );
                        // Just drop the stream to reject
                        continue;
                    }

                    // Allocate a token for this connection
                    let token = Token(self.next_token);
                    self.next_token += 1;

                    // Register the stream
                    self.poll
                        .registry()
                        .register(&mut stream, token, Interest::READABLE)?;

                    // Create the connection (with rate limiting if configured)
                    let conn = if let Some(rate_config) = self.config.rate_limit {
                        Connection::with_rate_limit(
                            token,
                            stream,
                            self.config.read_buffer_size,
                            rate_config,
                        )
                    } else {
                        Connection::new(token, stream, self.config.read_buffer_size)
                    };
                    self.connections.insert(token, conn);

                    debug!("Accepted connection from {} (token {:?})", addr, token);
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // No more connections to accept
                    break;
                }
                Err(e) => {
                    error!("Error accepting connection: {}", e);
                    break;
                }
            }
        }
        Ok(())
    }

    /// Handles readable events on a connection.
    fn handle_readable(&mut self, token: Token) -> ServerResult<()> {
        let Some(conn) = self.connections.get_mut(&token) else {
            warn!("Readable event for unknown token {:?}", token);
            return Ok(());
        };

        // Update activity timestamp
        conn.touch();

        // Read data from the socket
        match conn.read() {
            Ok(true) => {
                // Connection still open, process requests
                self.process_requests(token);
            }
            Ok(false) => {
                // Connection closed by peer
                debug!("Connection {:?} closed by peer", token);
                if let Some(c) = self.connections.get_mut(&token) {
                    c.closing = true;
                }
            }
            Err(e) => {
                error!("Error reading from {:?}: {}", token, e);
                if let Some(c) = self.connections.get_mut(&token) {
                    c.closing = true;
                }
            }
        }

        // Update interest if needed
        self.update_interest(token)?;
        Ok(())
    }

    /// Handles writable events on a connection.
    fn handle_writable(&mut self, token: Token) -> ServerResult<()> {
        let Some(conn) = self.connections.get_mut(&token) else {
            warn!("Writable event for unknown token {:?}", token);
            return Ok(());
        };

        match conn.write() {
            Ok(true) => {
                // All data written
                trace!("All data written to {:?}", token);
            }
            Ok(false) => {
                // More data to write
                trace!("More data to write to {:?}", token);
            }
            Err(e) => {
                error!("Error writing to {:?}: {}", token, e);
                conn.closing = true;
            }
        }

        // Update interest
        self.update_interest(token)?;
        Ok(())
    }

    /// Processes pending requests on a connection.
    fn process_requests(&mut self, token: Token) {
        use vdb_wire::{ErrorCode, Response};

        loop {
            let Some(conn) = self.connections.get_mut(&token) else {
                return;
            };

            // Check if there's enough data for a frame
            if !conn.has_pending_data() {
                break;
            }

            // Try to decode a request
            match conn.try_decode_request() {
                Ok(Some(request)) => {
                    trace!("Received request {:?} from {:?}", request.id, token);

                    // Check rate limit before processing
                    let Some(conn) = self.connections.get_mut(&token) else {
                        return;
                    };

                    if !conn.check_rate_limit() {
                        warn!("Rate limit exceeded for {:?}", token);
                        let response = Response::error(
                            request.id,
                            ErrorCode::RateLimited,
                            "rate limit exceeded".to_string(),
                        );
                        if let Err(e) = conn.queue_response(&response) {
                            error!("Error encoding rate limit response: {}", e);
                            conn.closing = true;
                        }
                        continue;
                    }

                    // Handle the request
                    let response = self.handler.handle(request);

                    // Queue the response
                    if let Some(c) = self.connections.get_mut(&token) {
                        if let Err(e) = c.queue_response(&response) {
                            error!("Error encoding response: {}", e);
                            c.closing = true;
                        }
                    }
                }
                Ok(None) => {
                    // Need more data
                    break;
                }
                Err(e) => {
                    error!("Error decoding request from {:?}: {}", token, e);
                    if let Some(c) = self.connections.get_mut(&token) {
                        c.closing = true;
                    }
                    break;
                }
            }
        }
    }

    /// Updates the interest flags for a connection.
    fn update_interest(&mut self, token: Token) -> ServerResult<()> {
        let Some(conn) = self.connections.get_mut(&token) else {
            return Ok(());
        };

        let interest = conn.interest();
        self.poll
            .registry()
            .reregister(&mut conn.stream, token, interest)?;

        Ok(())
    }

    /// Cleans up connections that have been marked as closing or are idle.
    fn cleanup_closed(&mut self) {
        let idle_timeout = self.config.idle_timeout;

        let to_close: Vec<Token> = self
            .connections
            .iter()
            .filter(|(_, c)| {
                if c.closing {
                    return true;
                }
                // Check idle timeout
                if let Some(timeout) = idle_timeout {
                    if c.is_idle(timeout) {
                        return true;
                    }
                }
                false
            })
            .map(|(t, _)| *t)
            .collect();

        for token in to_close {
            if let Some(mut conn) = self.connections.remove(&token) {
                if conn.closing {
                    debug!("Closing connection {:?}", token);
                } else {
                    debug!("Closing idle connection {:?}", token);
                }
                let _ = self.poll.registry().deregister(&mut conn.stream);
            }
        }
    }

    /// Returns the number of active connections.
    pub fn connection_count(&self) -> usize {
        self.connections.len()
    }
}
