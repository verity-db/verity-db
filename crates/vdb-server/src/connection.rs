//! Connection state management.

use std::io::{self, Read, Write};
use std::time::Instant;

use bytes::BytesMut;
use mio::net::TcpStream;
use mio::{Interest, Token};

use vdb_wire::{FRAME_HEADER_SIZE, Frame, Request, Response};

use crate::config::RateLimitConfig;
use crate::error::ServerResult;

/// State of a client connection.
pub struct Connection {
    /// Unique token for this connection (kept for debugging).
    #[allow(dead_code)]
    pub token: Token,
    /// TCP stream.
    pub stream: TcpStream,
    /// Read buffer.
    pub read_buf: BytesMut,
    /// Write buffer.
    pub write_buf: BytesMut,
    /// Whether the connection is closing.
    pub closing: bool,
    /// Last activity timestamp for idle timeout tracking.
    pub last_activity: Instant,
    /// Rate limiting state.
    pub rate_limiter: Option<RateLimiter>,
}

/// Simple sliding window rate limiter.
pub struct RateLimiter {
    config: RateLimitConfig,
    /// Request timestamps in the current window.
    request_times: Vec<Instant>,
}

impl RateLimiter {
    /// Creates a new rate limiter with the given configuration.
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            config,
            request_times: Vec::new(),
        }
    }

    /// Checks if a request should be allowed.
    ///
    /// Returns `true` if the request is allowed, `false` if rate limited.
    pub fn check(&mut self) -> bool {
        let now = Instant::now();

        // Remove old requests outside the window
        self.request_times
            .retain(|&t| now.duration_since(t) < self.config.window);

        // Check if under limit
        if self.request_times.len() < self.config.max_requests as usize {
            self.request_times.push(now);
            true
        } else {
            false
        }
    }

    /// Returns the number of requests in the current window.
    #[allow(dead_code)] // Useful for debugging/monitoring
    pub fn current_count(&self) -> usize {
        let now = Instant::now();
        self.request_times
            .iter()
            .filter(|&&t| now.duration_since(t) < self.config.window)
            .count()
    }
}

impl Connection {
    /// Creates a new connection.
    pub fn new(token: Token, stream: TcpStream, buffer_size: usize) -> Self {
        Self {
            token,
            stream,
            read_buf: BytesMut::with_capacity(buffer_size),
            write_buf: BytesMut::with_capacity(buffer_size),
            closing: false,
            last_activity: Instant::now(),
            rate_limiter: None,
        }
    }

    /// Creates a new connection with rate limiting.
    pub fn with_rate_limit(
        token: Token,
        stream: TcpStream,
        buffer_size: usize,
        rate_config: RateLimitConfig,
    ) -> Self {
        Self {
            token,
            stream,
            read_buf: BytesMut::with_capacity(buffer_size),
            write_buf: BytesMut::with_capacity(buffer_size),
            closing: false,
            last_activity: Instant::now(),
            rate_limiter: Some(RateLimiter::new(rate_config)),
        }
    }

    /// Updates the last activity timestamp.
    pub fn touch(&mut self) {
        self.last_activity = Instant::now();
    }

    /// Checks if the connection has been idle for longer than the timeout.
    pub fn is_idle(&self, timeout: std::time::Duration) -> bool {
        self.last_activity.elapsed() > timeout
    }

    /// Checks if a request should be rate limited.
    ///
    /// Returns `true` if the request is allowed, `false` if rate limited.
    pub fn check_rate_limit(&mut self) -> bool {
        match &mut self.rate_limiter {
            Some(limiter) => limiter.check(),
            None => true, // No rate limiting configured
        }
    }

    /// Reads data from the socket into the read buffer.
    ///
    /// Returns `true` if the connection is still open.
    pub fn read(&mut self) -> io::Result<bool> {
        // Use a temporary stack buffer to avoid unsafe
        let mut temp_buf = [0u8; 4096];

        loop {
            match self.stream.read(&mut temp_buf) {
                Ok(0) => {
                    // Connection closed
                    return Ok(false);
                }
                Ok(n) => {
                    self.read_buf.extend_from_slice(&temp_buf[..n]);
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    // No more data available
                    return Ok(true);
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Writes data from the write buffer to the socket.
    ///
    /// Returns `true` if all data was written.
    pub fn write(&mut self) -> io::Result<bool> {
        while !self.write_buf.is_empty() {
            match self.stream.write(&self.write_buf) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "failed to write to socket",
                    ));
                }
                Ok(n) => {
                    let _ = self.write_buf.split_to(n);
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    // Socket not ready for writing
                    return Ok(false);
                }
                Err(e) => return Err(e),
            }
        }
        Ok(true)
    }

    /// Attempts to decode a request from the read buffer.
    pub fn try_decode_request(&mut self) -> ServerResult<Option<Request>> {
        // Try to decode a frame
        let frame = Frame::decode(&mut self.read_buf)?;

        match frame {
            Some(f) => {
                // Decode the request from the frame
                let request = Request::from_frame(&f)?;
                Ok(Some(request))
            }
            None => Ok(None),
        }
    }

    /// Queues a response to be sent.
    pub fn queue_response(&mut self, response: &Response) -> ServerResult<()> {
        let frame = response.to_frame()?;
        frame.encode(&mut self.write_buf);
        Ok(())
    }

    /// Returns the interest flags for this connection.
    pub fn interest(&self) -> Interest {
        if self.write_buf.is_empty() {
            Interest::READABLE
        } else {
            Interest::READABLE | Interest::WRITABLE
        }
    }

    /// Returns true if there's pending data to process.
    pub fn has_pending_data(&self) -> bool {
        self.read_buf.len() >= FRAME_HEADER_SIZE
    }
}
