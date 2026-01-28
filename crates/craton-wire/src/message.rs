//! Request and response message types for the wire protocol.
//!
//! Messages are serialized using bincode for efficient binary encoding.

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use craton_types::{DataClass, Offset, Placement, StreamId, TenantId};

use crate::error::{WireError, WireResult};
use crate::frame::Frame;

/// Unique identifier for a request, used to match responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RequestId(pub u64);

impl RequestId {
    /// Creates a new request ID.
    pub fn new(id: u64) -> Self {
        Self(id)
    }
}

// ============================================================================
// Request Types
// ============================================================================

/// A client request to the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    /// Unique request identifier.
    pub id: RequestId,
    /// Tenant context for the request.
    pub tenant_id: TenantId,
    /// The request payload.
    pub payload: RequestPayload,
}

impl Request {
    /// Creates a new request.
    pub fn new(id: RequestId, tenant_id: TenantId, payload: RequestPayload) -> Self {
        Self {
            id,
            tenant_id,
            payload,
        }
    }

    /// Encodes the request to a frame.
    pub fn to_frame(&self) -> WireResult<Frame> {
        let payload =
            bincode::serialize(self).map_err(|e| WireError::Serialization(e.to_string()))?;
        Ok(Frame::new(Bytes::from(payload)))
    }

    /// Decodes a request from a frame.
    pub fn from_frame(frame: &Frame) -> WireResult<Self> {
        bincode::deserialize(&frame.payload).map_err(WireError::from)
    }
}

/// Request payload variants.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RequestPayload {
    /// Handshake to establish connection.
    Handshake(HandshakeRequest),
    /// Create a new stream.
    CreateStream(CreateStreamRequest),
    /// Append events to a stream.
    AppendEvents(AppendEventsRequest),
    /// Execute a SQL query.
    Query(QueryRequest),
    /// Execute a SQL query at a specific position.
    QueryAt(QueryAtRequest),
    /// Read events from a stream.
    ReadEvents(ReadEventsRequest),
    /// Sync all data to disk.
    Sync(SyncRequest),
}

/// Handshake request to establish connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandshakeRequest {
    /// Client protocol version.
    pub client_version: u16,
    /// Optional authentication token.
    pub auth_token: Option<String>,
}

/// Create stream request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateStreamRequest {
    /// Stream name.
    pub name: String,
    /// Data classification.
    pub data_class: DataClass,
    /// Placement policy.
    pub placement: Placement,
}

/// Append events request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppendEventsRequest {
    /// Target stream.
    pub stream_id: StreamId,
    /// Events to append.
    pub events: Vec<Vec<u8>>,
}

/// SQL query request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryRequest {
    /// SQL query string.
    pub sql: String,
    /// Query parameters.
    pub params: Vec<QueryParam>,
}

/// SQL query at specific position request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryAtRequest {
    /// SQL query string.
    pub sql: String,
    /// Query parameters.
    pub params: Vec<QueryParam>,
    /// Log position to query at.
    pub position: Offset,
}

/// Query parameter value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum QueryParam {
    /// Null value.
    Null,
    /// 64-bit integer.
    BigInt(i64),
    /// Text string.
    Text(String),
    /// Boolean.
    Boolean(bool),
    /// Timestamp (nanoseconds since epoch).
    Timestamp(i64),
}

/// Read events request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadEventsRequest {
    /// Source stream.
    pub stream_id: StreamId,
    /// Starting offset (inclusive).
    pub from_offset: Offset,
    /// Maximum bytes to read.
    pub max_bytes: u64,
}

/// Sync request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncRequest {}

// ============================================================================
// Response Types
// ============================================================================

/// A server response to a client request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    /// Request ID this is responding to.
    pub request_id: RequestId,
    /// The response payload.
    pub payload: ResponsePayload,
}

impl Response {
    /// Creates a new response.
    pub fn new(request_id: RequestId, payload: ResponsePayload) -> Self {
        Self {
            request_id,
            payload,
        }
    }

    /// Creates an error response.
    pub fn error(request_id: RequestId, code: ErrorCode, message: String) -> Self {
        Self {
            request_id,
            payload: ResponsePayload::Error(ErrorResponse { code, message }),
        }
    }

    /// Encodes the response to a frame.
    pub fn to_frame(&self) -> WireResult<Frame> {
        let payload =
            bincode::serialize(self).map_err(|e| WireError::Serialization(e.to_string()))?;
        Ok(Frame::new(Bytes::from(payload)))
    }

    /// Decodes a response from a frame.
    pub fn from_frame(frame: &Frame) -> WireResult<Self> {
        bincode::deserialize(&frame.payload).map_err(WireError::from)
    }
}

/// Response payload variants.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ResponsePayload {
    /// Error response.
    Error(ErrorResponse),
    /// Handshake response.
    Handshake(HandshakeResponse),
    /// Create stream response.
    CreateStream(CreateStreamResponse),
    /// Append events response.
    AppendEvents(AppendEventsResponse),
    /// Query response.
    Query(QueryResponse),
    /// Query at response.
    QueryAt(QueryAtResponse),
    /// Read events response.
    ReadEvents(ReadEventsResponse),
    /// Sync response.
    Sync(SyncResponse),
}

/// Error response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    /// Error code.
    pub code: ErrorCode,
    /// Human-readable error message.
    pub message: String,
}

/// Error codes for wire protocol errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u16)]
pub enum ErrorCode {
    /// Unknown error.
    Unknown = 0,
    /// Internal server error.
    InternalError = 1,
    /// Invalid request format.
    InvalidRequest = 2,
    /// Authentication failed.
    AuthenticationFailed = 3,
    /// Tenant not found.
    TenantNotFound = 4,
    /// Stream not found.
    StreamNotFound = 5,
    /// Table not found.
    TableNotFound = 6,
    /// Query parse error.
    QueryParseError = 7,
    /// Query execution error.
    QueryExecutionError = 8,
    /// Position ahead of current.
    PositionAhead = 9,
    /// Stream already exists.
    StreamAlreadyExists = 10,
    /// Invalid stream offset.
    InvalidOffset = 11,
    /// Storage error.
    StorageError = 12,
    /// Projection lag.
    ProjectionLag = 13,
    /// Rate limit exceeded.
    RateLimited = 14,
}

/// Handshake response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandshakeResponse {
    /// Server protocol version.
    pub server_version: u16,
    /// Whether authentication succeeded.
    pub authenticated: bool,
    /// Server capabilities.
    pub capabilities: Vec<String>,
}

/// Create stream response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateStreamResponse {
    /// The created stream ID.
    pub stream_id: StreamId,
}

/// Append events response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppendEventsResponse {
    /// Offset of the first appended event.
    pub first_offset: Offset,
    /// Number of events appended.
    pub count: u32,
}

/// Query response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResponse {
    /// Column names.
    pub columns: Vec<String>,
    /// Rows of data.
    pub rows: Vec<Vec<QueryValue>>,
}

/// Query at response (same as Query).
pub type QueryAtResponse = QueryResponse;

/// Query result value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum QueryValue {
    /// Null value.
    Null,
    /// 64-bit integer.
    BigInt(i64),
    /// Text string.
    Text(String),
    /// Boolean.
    Boolean(bool),
    /// Timestamp (nanoseconds since epoch).
    Timestamp(i64),
}

/// Read events response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadEventsResponse {
    /// The events.
    pub events: Vec<Vec<u8>>,
    /// Next offset to read from (for pagination).
    pub next_offset: Option<Offset>,
}

/// Sync response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResponse {
    /// Whether sync completed successfully.
    pub success: bool,
}

#[cfg(test)]
mod message_tests {
    use super::*;

    #[test]
    fn test_request_roundtrip() {
        let request = Request::new(
            RequestId::new(1),
            TenantId::new(42),
            RequestPayload::CreateStream(CreateStreamRequest {
                name: "test-stream".to_string(),
                data_class: DataClass::NonPHI,
                placement: Placement::Global,
            }),
        );

        // Encode to frame
        let frame = request.to_frame().unwrap();

        // Decode from frame
        let decoded = Request::from_frame(&frame).unwrap();

        assert_eq!(decoded.id, request.id);
        assert_eq!(u64::from(decoded.tenant_id), 42);
    }

    #[test]
    fn test_response_roundtrip() {
        let response = Response::new(
            RequestId::new(1),
            ResponsePayload::CreateStream(CreateStreamResponse {
                stream_id: StreamId::new(100),
            }),
        );

        // Encode to frame
        let frame = response.to_frame().unwrap();

        // Decode from frame
        let decoded = Response::from_frame(&frame).unwrap();

        assert_eq!(decoded.request_id, response.request_id);
    }

    #[test]
    fn test_error_response() {
        let response = Response::error(
            RequestId::new(1),
            ErrorCode::StreamNotFound,
            "stream 123 not found".to_string(),
        );

        let frame = response.to_frame().unwrap();
        let decoded = Response::from_frame(&frame).unwrap();

        if let ResponsePayload::Error(err) = decoded.payload {
            assert_eq!(err.code, ErrorCode::StreamNotFound);
            assert_eq!(err.message, "stream 123 not found");
        } else {
            panic!("expected error payload");
        }
    }

    #[test]
    fn test_query_params() {
        let request = Request::new(
            RequestId::new(2),
            TenantId::new(1),
            RequestPayload::Query(QueryRequest {
                sql: "SELECT * FROM events WHERE id = $1".to_string(),
                params: vec![
                    QueryParam::BigInt(42),
                    QueryParam::Text("hello".to_string()),
                    QueryParam::Boolean(true),
                    QueryParam::Null,
                ],
            }),
        );

        let frame = request.to_frame().unwrap();
        let decoded = Request::from_frame(&frame).unwrap();

        if let RequestPayload::Query(q) = decoded.payload {
            assert_eq!(q.params.len(), 4);
        } else {
            panic!("expected query payload");
        }
    }
}
