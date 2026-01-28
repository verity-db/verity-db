//! Server configuration.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

/// Server configuration.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Address to bind to.
    pub bind_addr: SocketAddr,
    /// Path to the data directory.
    pub data_dir: PathBuf,
    /// Maximum number of concurrent connections.
    pub max_connections: usize,
    /// Read buffer size per connection.
    pub read_buffer_size: usize,
    /// Write buffer size per connection.
    pub write_buffer_size: usize,
    /// Idle connection timeout. Connections with no activity for this
    /// duration will be closed. Set to None to disable.
    pub idle_timeout: Option<Duration>,
    /// Maximum requests per connection per minute for rate limiting.
    /// Set to None to disable rate limiting.
    pub rate_limit: Option<RateLimitConfig>,
}

/// Rate limiting configuration.
#[derive(Debug, Clone, Copy)]
pub struct RateLimitConfig {
    /// Maximum requests per window.
    pub max_requests: u32,
    /// Window duration.
    pub window: Duration,
}

impl ServerConfig {
    /// Creates a new server configuration.
    pub fn new(bind_addr: impl Into<SocketAddr>, data_dir: impl Into<PathBuf>) -> Self {
        Self {
            bind_addr: bind_addr.into(),
            data_dir: data_dir.into(),
            max_connections: 1024,
            read_buffer_size: 64 * 1024,  // 64 KiB
            write_buffer_size: 64 * 1024, // 64 KiB
            idle_timeout: Some(Duration::from_secs(300)), // 5 minutes default
            rate_limit: None,
        }
    }

    /// Sets the maximum number of concurrent connections.
    pub fn with_max_connections(mut self, max: usize) -> Self {
        self.max_connections = max;
        self
    }

    /// Sets the read buffer size.
    pub fn with_read_buffer_size(mut self, size: usize) -> Self {
        self.read_buffer_size = size;
        self
    }

    /// Sets the write buffer size.
    pub fn with_write_buffer_size(mut self, size: usize) -> Self {
        self.write_buffer_size = size;
        self
    }

    /// Sets the idle connection timeout.
    ///
    /// Connections with no activity for this duration will be closed.
    pub fn with_idle_timeout(mut self, timeout: Duration) -> Self {
        self.idle_timeout = Some(timeout);
        self
    }

    /// Disables idle timeout (connections never timeout).
    pub fn without_idle_timeout(mut self) -> Self {
        self.idle_timeout = None;
        self
    }

    /// Enables rate limiting.
    ///
    /// # Arguments
    ///
    /// * `max_requests` - Maximum requests per window
    /// * `window` - Time window for rate limiting
    pub fn with_rate_limit(mut self, max_requests: u32, window: Duration) -> Self {
        self.rate_limit = Some(RateLimitConfig { max_requests, window });
        self
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: "127.0.0.1:5432".parse().expect("valid address"),
            data_dir: PathBuf::from("./data"),
            max_connections: 1024,
            read_buffer_size: 64 * 1024,
            write_buffer_size: 64 * 1024,
            idle_timeout: Some(Duration::from_secs(300)),
            rate_limit: None,
        }
    }
}

impl RateLimitConfig {
    /// Creates a new rate limit configuration.
    pub fn new(max_requests: u32, window: Duration) -> Self {
        Self { max_requests, window }
    }

    /// Creates a rate limit of N requests per minute.
    pub fn per_minute(max_requests: u32) -> Self {
        Self::new(max_requests, Duration::from_secs(60))
    }

    /// Creates a rate limit of N requests per second.
    pub fn per_second(max_requests: u32) -> Self {
        Self::new(max_requests, Duration::from_secs(1))
    }
}
