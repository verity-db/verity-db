//! # craton-client: RPC client for `Craton`
//!
//! This crate provides a synchronous RPC client for communicating with
//! a `Craton` server using the binary wire protocol defined in `craton-wire`.
//!
//! ## Usage
//!
//! ```ignore
//! use craton_client::{Client, ClientConfig};
//! use craton_types::{DataClass, TenantId};
//!
//! // Connect to server
//! let mut client = Client::connect(
//!     "127.0.0.1:5432",
//!     TenantId::new(1),
//!     ClientConfig::default(),
//! )?;
//!
//! // Create a stream
//! let stream_id = client.create_stream("events", DataClass::NonPHI)?;
//!
//! // Append events
//! let offset = client.append(stream_id, vec![
//!     b"event1".to_vec(),
//!     b"event2".to_vec(),
//! ])?;
//!
//! // Read events back
//! let events = client.read_events(stream_id, craton_types::Offset::new(0), 1024)?;
//!
//! // Execute a query
//! let result = client.query("SELECT * FROM streams", &[])?;
//! ```
//!
//! ## Configuration
//!
//! The client can be configured with timeouts and buffer sizes:
//!
//! ```ignore
//! use craton_client::ClientConfig;
//! use std::time::Duration;
//!
//! let config = ClientConfig {
//!     read_timeout: Some(Duration::from_secs(60)),
//!     write_timeout: Some(Duration::from_secs(30)),
//!     buffer_size: 128 * 1024,
//!     auth_token: Some("secret-token".to_string()),
//! };
//! ```

mod client;
mod error;

pub use client::{Client, ClientConfig};
pub use error::{ClientError, ClientResult};

// Re-export useful types from dependencies
pub use craton_wire::{QueryParam, QueryResponse, QueryValue, ReadEventsResponse};
