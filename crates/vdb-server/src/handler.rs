//! Request handler that routes requests to Verity.

use vdb::{Offset, Verity};
use vdb_query::Value;
use vdb_types::Timestamp;
use vdb_wire::{
    AppendEventsResponse, CreateStreamResponse, ErrorCode, ErrorResponse, HandshakeResponse,
    PROTOCOL_VERSION, QueryParam, QueryResponse, QueryValue, ReadEventsResponse, Request,
    RequestPayload, Response, ResponsePayload, SyncResponse,
};

use crate::error::{ServerError, ServerResult};
use crate::replication::CommandSubmitter;

/// Handles requests by routing them to the appropriate Verity operations.
pub struct RequestHandler {
    /// The command submitter (wraps Verity with optional replication).
    submitter: CommandSubmitter,
}

impl RequestHandler {
    /// Creates a new request handler with a command submitter.
    pub fn new(submitter: CommandSubmitter) -> Self {
        Self { submitter }
    }

    /// Creates a new request handler with direct Verity access (no replication).
    #[allow(dead_code)] // Available for direct testing without replication
    pub fn new_direct(db: Verity) -> Self {
        Self {
            submitter: CommandSubmitter::Direct { db },
        }
    }

    /// Returns a reference to the underlying Verity instance.
    pub fn verity(&self) -> &Verity {
        self.submitter.verity()
    }

    /// Handles a request and returns a response.
    pub fn handle(&self, request: Request) -> Response {
        let request_id = request.id;

        match self.handle_inner(request) {
            Ok(payload) => Response::new(request_id, payload),
            Err(e) => {
                let (code, message) = error_to_wire(&e);
                Response::error(request_id, code, message)
            }
        }
    }

    fn handle_inner(&self, request: Request) -> ServerResult<ResponsePayload> {
        let tenant = self.verity().tenant(request.tenant_id);

        match request.payload {
            RequestPayload::Handshake(req) => {
                // Version check
                if req.client_version != PROTOCOL_VERSION {
                    return Ok(ResponsePayload::Error(ErrorResponse {
                        code: ErrorCode::InvalidRequest,
                        message: format!(
                            "unsupported client version: {}, server is {}",
                            req.client_version, PROTOCOL_VERSION
                        ),
                    }));
                }

                Ok(ResponsePayload::Handshake(HandshakeResponse {
                    server_version: PROTOCOL_VERSION,
                    authenticated: req.auth_token.is_some(), // TODO: Real auth
                    capabilities: vec!["query".to_string(), "append".to_string()],
                }))
            }

            RequestPayload::CreateStream(req) => {
                let stream_id = tenant.create_stream(&req.name, req.data_class)?;
                Ok(ResponsePayload::CreateStream(CreateStreamResponse {
                    stream_id,
                }))
            }

            RequestPayload::AppendEvents(req) => {
                let first_offset = tenant.append(req.stream_id, req.events.clone())?;
                Ok(ResponsePayload::AppendEvents(AppendEventsResponse {
                    first_offset,
                    count: req.events.len() as u32,
                }))
            }

            RequestPayload::Query(req) => {
                let params = convert_params(&req.params);
                let result = tenant.query(&req.sql, &params)?;

                Ok(ResponsePayload::Query(convert_query_result(&result)))
            }

            RequestPayload::QueryAt(req) => {
                let params = convert_params(&req.params);
                let result = tenant.query_at(&req.sql, &params, req.position)?;

                Ok(ResponsePayload::QueryAt(convert_query_result(&result)))
            }

            RequestPayload::ReadEvents(req) => {
                let events = tenant.read_events(req.stream_id, req.from_offset, req.max_bytes)?;

                // Calculate next offset for pagination
                let next_offset = if events.is_empty() {
                    None
                } else {
                    Some(Offset::new(req.from_offset.as_u64() + events.len() as u64))
                };

                Ok(ResponsePayload::ReadEvents(ReadEventsResponse {
                    events: events.into_iter().map(|b| b.to_vec()).collect(),
                    next_offset,
                }))
            }

            RequestPayload::Sync(_) => {
                self.verity().sync()?;
                Ok(ResponsePayload::Sync(SyncResponse { success: true }))
            }
        }
    }
}

/// Converts wire query parameters to Verity query values.
fn convert_params(params: &[QueryParam]) -> Vec<Value> {
    params
        .iter()
        .map(|p| match p {
            QueryParam::Null => Value::Null,
            QueryParam::BigInt(v) => Value::BigInt(*v),
            QueryParam::Text(v) => Value::Text(v.clone()),
            QueryParam::Boolean(v) => Value::Boolean(*v),
            // Negative timestamps are treated as 0 (epoch)
            QueryParam::Timestamp(v) => {
                #[allow(clippy::cast_sign_loss)]
                let nanos = if *v < 0 { 0 } else { *v as u64 };
                Value::Timestamp(Timestamp::from_nanos(nanos))
            }
        })
        .collect()
}

/// Converts a Verity query result to a wire response.
fn convert_query_result(result: &vdb_query::QueryResult) -> QueryResponse {
    let columns = result.columns.iter().map(ToString::to_string).collect();

    let rows = result
        .rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|v| match v {
                    Value::Null => QueryValue::Null,
                    Value::BigInt(n) => QueryValue::BigInt(*n),
                    Value::Text(s) => QueryValue::Text(s.clone()),
                    Value::Boolean(b) => QueryValue::Boolean(*b),
                    // Timestamps are transmitted as i64 (may overflow for very large values)
                    #[allow(clippy::cast_possible_wrap)]
                    Value::Timestamp(t) => QueryValue::Timestamp(t.as_nanos() as i64),
                    Value::Bytes(b) => {
                        // Encode bytes as base64 text for wire transmission
                        use base64::Engine;
                        let encoded = base64::engine::general_purpose::STANDARD.encode(b);
                        QueryValue::Text(encoded)
                    }
                })
                .collect()
        })
        .collect();

    QueryResponse { columns, rows }
}

/// Converts a server error to a wire error code and message.
fn error_to_wire(error: &ServerError) -> (ErrorCode, String) {
    match error {
        ServerError::Wire(e) => (ErrorCode::InvalidRequest, e.to_string()),
        ServerError::Database(e) => match e {
            vdb::VerityError::TenantNotFound(_) => (ErrorCode::TenantNotFound, e.to_string()),
            vdb::VerityError::StreamNotFound(_) => (ErrorCode::StreamNotFound, e.to_string()),
            vdb::VerityError::TableNotFound(_) => (ErrorCode::TableNotFound, e.to_string()),
            vdb::VerityError::PositionAhead { .. } => (ErrorCode::PositionAhead, e.to_string()),
            vdb::VerityError::ProjectionLag { .. } => (ErrorCode::ProjectionLag, e.to_string()),
            vdb::VerityError::Query(qe) => (ErrorCode::QueryParseError, qe.to_string()),
            vdb::VerityError::Storage(_) | vdb::VerityError::Store(_) => {
                (ErrorCode::StorageError, e.to_string())
            }
            vdb::VerityError::Kernel(ke) => {
                // Map kernel errors to appropriate wire codes
                let msg = ke.to_string();
                if msg.contains("not found") {
                    (ErrorCode::StreamNotFound, msg)
                } else if msg.contains("already exists") || msg.contains("unique") {
                    (ErrorCode::StreamAlreadyExists, msg)
                } else if msg.contains("offset") {
                    (ErrorCode::InvalidOffset, msg)
                } else {
                    (ErrorCode::InternalError, msg)
                }
            }
            _ => (ErrorCode::InternalError, e.to_string()),
        },
        ServerError::Io(e) => (ErrorCode::InternalError, e.to_string()),
        ServerError::ConnectionClosed => {
            (ErrorCode::InternalError, "connection closed".to_string())
        }
        ServerError::MaxConnectionsReached(n) => (
            ErrorCode::InternalError,
            format!("max connections reached: {n}"),
        ),
        ServerError::InvalidTenant => (ErrorCode::TenantNotFound, "invalid tenant".to_string()),
        ServerError::BindFailed { addr, source } => (
            ErrorCode::InternalError,
            format!("bind failed on {addr}: {source}"),
        ),
        ServerError::Tls(msg) => (ErrorCode::InternalError, format!("TLS error: {msg}")),
        ServerError::Unauthorized(msg) => (ErrorCode::AuthenticationFailed, msg.clone()),
        ServerError::Shutdown => (ErrorCode::InternalError, "server shutdown".to_string()),
        ServerError::Replication(msg) => (ErrorCode::InternalError, format!("replication: {msg}")),
    }
}
