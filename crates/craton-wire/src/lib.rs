//! # craton-wire: Binary wire protocol for `Craton`
//!
//! This crate defines the binary wire protocol used for client-server
//! communication in `Craton`.
//!
//! ## Frame Format
//!
//! ```text
//! ┌─────────┬─────────┬──────────┬──────────┬──────────────────┐
//! │ Magic   │ Version │ Length   │ Checksum │     Payload      │
//! │ (4 B)   │ (2 B)   │ (4 B)    │ (4 B)    │     (var)        │
//! └─────────┴─────────┴──────────┴──────────┴──────────────────┘
//! ```
//!
//! - **Magic**: `0x56444220` ("VDB ")
//! - **Version**: Protocol version (currently 1)
//! - **Length**: Payload length in bytes (max 16 MiB)
//! - **Checksum**: CRC32 of payload
//! - **Payload**: Bincode-encoded message
//!
//! ## Message Types
//!
//! Messages are either requests (client → server) or responses (server → client).
//! Each request has a corresponding response type.

mod error;
mod frame;
mod message;

pub use error::{WireError, WireResult};
pub use frame::{FRAME_HEADER_SIZE, Frame, FrameHeader, MAGIC, MAX_PAYLOAD_SIZE, PROTOCOL_VERSION};
pub use message::{
    AppendEventsRequest, AppendEventsResponse, CreateStreamRequest, CreateStreamResponse,
    ErrorCode, ErrorResponse, HandshakeRequest, HandshakeResponse, QueryAtRequest, QueryAtResponse,
    QueryParam, QueryRequest, QueryResponse, QueryValue, ReadEventsRequest, ReadEventsResponse,
    Request, RequestId, RequestPayload, Response, ResponsePayload, SyncRequest, SyncResponse,
};

#[cfg(test)]
mod tests;
