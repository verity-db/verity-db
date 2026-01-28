//! RPC client for `Craton`.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use bytes::BytesMut;
use craton_types::{DataClass, Offset, Placement, StreamId, TenantId};
use craton_wire::{
    AppendEventsRequest, CreateStreamRequest, ErrorCode, Frame, HandshakeRequest, PROTOCOL_VERSION,
    QueryAtRequest, QueryParam, QueryRequest, QueryResponse, ReadEventsRequest, ReadEventsResponse,
    Request, RequestId, RequestPayload, Response, ResponsePayload, SyncRequest,
};

use crate::error::{ClientError, ClientResult};

/// Configuration for the client.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Read timeout.
    pub read_timeout: Option<Duration>,
    /// Write timeout.
    pub write_timeout: Option<Duration>,
    /// Buffer size for reads.
    pub buffer_size: usize,
    /// Authentication token.
    pub auth_token: Option<String>,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            read_timeout: Some(Duration::from_secs(30)),
            write_timeout: Some(Duration::from_secs(30)),
            buffer_size: 64 * 1024,
            auth_token: None,
        }
    }
}

/// RPC client for `Craton`.
///
/// This client uses synchronous I/O to communicate with a `Craton` server
/// using the binary wire protocol.
///
/// # Example
///
/// ```ignore
/// use craton_client::{Client, ClientConfig};
/// use craton_types::{DataClass, TenantId};
///
/// let mut client = Client::connect("127.0.0.1:5432", TenantId::new(1), ClientConfig::default())?;
///
/// // Create a stream
/// let stream_id = client.create_stream("events", DataClass::NonPHI)?;
///
/// // Append events
/// let offset = client.append(stream_id, vec![b"event1".to_vec(), b"event2".to_vec()])?;
/// ```
pub struct Client {
    stream: TcpStream,
    tenant_id: TenantId,
    next_request_id: u64,
    read_buf: BytesMut,
    config: ClientConfig,
}

impl Client {
    /// Connects to a `Craton` server.
    pub fn connect(
        addr: impl ToSocketAddrs,
        tenant_id: TenantId,
        config: ClientConfig,
    ) -> ClientResult<Self> {
        let stream = TcpStream::connect(addr)?;
        stream.set_read_timeout(config.read_timeout)?;
        stream.set_write_timeout(config.write_timeout)?;

        let mut client = Self {
            stream,
            tenant_id,
            next_request_id: 1,
            read_buf: BytesMut::with_capacity(config.buffer_size),
            config,
        };

        // Perform handshake
        client.handshake()?;

        Ok(client)
    }

    /// Performs the handshake with the server.
    fn handshake(&mut self) -> ClientResult<()> {
        let response = self.send_request(RequestPayload::Handshake(HandshakeRequest {
            client_version: PROTOCOL_VERSION,
            auth_token: self.config.auth_token.clone(),
        }))?;

        match response.payload {
            ResponsePayload::Handshake(h) => {
                if h.server_version != PROTOCOL_VERSION {
                    return Err(ClientError::HandshakeFailed(format!(
                        "protocol version mismatch: client {}, server {}",
                        PROTOCOL_VERSION, h.server_version
                    )));
                }
                Ok(())
            }
            ResponsePayload::Error(e) => Err(ClientError::server(e.code, e.message)),
            _ => Err(ClientError::UnexpectedResponse {
                expected: "Handshake".to_string(),
                actual: format!("{:?}", response.payload),
            }),
        }
    }

    /// Creates a new stream.
    pub fn create_stream(&mut self, name: &str, data_class: DataClass) -> ClientResult<StreamId> {
        self.create_stream_with_placement(name, data_class, Placement::Global)
    }

    /// Creates a new stream with a specific placement policy.
    pub fn create_stream_with_placement(
        &mut self,
        name: &str,
        data_class: DataClass,
        placement: Placement,
    ) -> ClientResult<StreamId> {
        let response = self.send_request(RequestPayload::CreateStream(CreateStreamRequest {
            name: name.to_string(),
            data_class,
            placement,
        }))?;

        match response.payload {
            ResponsePayload::CreateStream(r) => Ok(r.stream_id),
            ResponsePayload::Error(e) => Err(ClientError::server(e.code, e.message)),
            _ => Err(ClientError::UnexpectedResponse {
                expected: "CreateStream".to_string(),
                actual: format!("{:?}", response.payload),
            }),
        }
    }

    /// Appends events to a stream.
    ///
    /// Returns the offset of the first appended event.
    pub fn append(&mut self, stream_id: StreamId, events: Vec<Vec<u8>>) -> ClientResult<Offset> {
        let response = self.send_request(RequestPayload::AppendEvents(AppendEventsRequest {
            stream_id,
            events,
        }))?;

        match response.payload {
            ResponsePayload::AppendEvents(r) => Ok(r.first_offset),
            ResponsePayload::Error(e) => Err(ClientError::server(e.code, e.message)),
            _ => Err(ClientError::UnexpectedResponse {
                expected: "AppendEvents".to_string(),
                actual: format!("{:?}", response.payload),
            }),
        }
    }

    /// Executes a SQL query.
    pub fn query(&mut self, sql: &str, params: &[QueryParam]) -> ClientResult<QueryResponse> {
        let response = self.send_request(RequestPayload::Query(QueryRequest {
            sql: sql.to_string(),
            params: params.to_vec(),
        }))?;

        match response.payload {
            ResponsePayload::Query(r) => Ok(r),
            ResponsePayload::Error(e) => Err(ClientError::server(e.code, e.message)),
            _ => Err(ClientError::UnexpectedResponse {
                expected: "Query".to_string(),
                actual: format!("{:?}", response.payload),
            }),
        }
    }

    /// Executes a SQL query at a specific position.
    pub fn query_at(
        &mut self,
        sql: &str,
        params: &[QueryParam],
        position: Offset,
    ) -> ClientResult<QueryResponse> {
        let response = self.send_request(RequestPayload::QueryAt(QueryAtRequest {
            sql: sql.to_string(),
            params: params.to_vec(),
            position,
        }))?;

        match response.payload {
            ResponsePayload::QueryAt(r) => Ok(r),
            ResponsePayload::Error(e) => Err(ClientError::server(e.code, e.message)),
            _ => Err(ClientError::UnexpectedResponse {
                expected: "QueryAt".to_string(),
                actual: format!("{:?}", response.payload),
            }),
        }
    }

    /// Reads events from a stream.
    pub fn read_events(
        &mut self,
        stream_id: StreamId,
        from_offset: Offset,
        max_bytes: u64,
    ) -> ClientResult<ReadEventsResponse> {
        let response = self.send_request(RequestPayload::ReadEvents(ReadEventsRequest {
            stream_id,
            from_offset,
            max_bytes,
        }))?;

        match response.payload {
            ResponsePayload::ReadEvents(r) => Ok(r),
            ResponsePayload::Error(e) => Err(ClientError::server(e.code, e.message)),
            _ => Err(ClientError::UnexpectedResponse {
                expected: "ReadEvents".to_string(),
                actual: format!("{:?}", response.payload),
            }),
        }
    }

    /// Syncs all data to disk.
    pub fn sync(&mut self) -> ClientResult<()> {
        let response = self.send_request(RequestPayload::Sync(SyncRequest {}))?;

        match response.payload {
            ResponsePayload::Sync(r) => {
                if r.success {
                    Ok(())
                } else {
                    Err(ClientError::server(ErrorCode::InternalError, "sync failed"))
                }
            }
            ResponsePayload::Error(e) => Err(ClientError::server(e.code, e.message)),
            _ => Err(ClientError::UnexpectedResponse {
                expected: "Sync".to_string(),
                actual: format!("{:?}", response.payload),
            }),
        }
    }

    /// Returns the tenant ID for this client.
    pub fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    /// Sends a request and waits for the response.
    fn send_request(&mut self, payload: RequestPayload) -> ClientResult<Response> {
        let request_id = RequestId::new(self.next_request_id);
        self.next_request_id += 1;

        let request = Request::new(request_id, self.tenant_id, payload);

        // Encode and send the request
        let frame = request.to_frame()?;
        let mut write_buf = BytesMut::new();
        frame.encode(&mut write_buf);
        self.stream.write_all(&write_buf)?;
        self.stream.flush()?;

        // Read the response
        let response = self.read_response()?;

        // Verify request ID matches
        if response.request_id.0 != request_id.0 {
            return Err(ClientError::ResponseMismatch {
                expected: request_id.0,
                received: response.request_id.0,
            });
        }

        Ok(response)
    }

    /// Reads a response from the server.
    fn read_response(&mut self) -> ClientResult<Response> {
        loop {
            // Try to decode a frame from the buffer
            if let Some(frame) = Frame::decode(&mut self.read_buf)? {
                let response = Response::from_frame(&frame)?;
                return Ok(response);
            }

            // Need more data - read from socket
            let mut temp_buf = [0u8; 4096];
            let n = self.stream.read(&mut temp_buf)?;
            if n == 0 {
                return Err(ClientError::Connection(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "server closed connection",
                )));
            }
            self.read_buf.extend_from_slice(&temp_buf[..n]);

            // Check for buffer overflow (simple DoS protection)
            if self.read_buf.len() > self.config.buffer_size * 2 {
                return Err(ClientError::Connection(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "response too large",
                )));
            }
        }
    }
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("tenant_id", &self.tenant_id)
            .field("next_request_id", &self.next_request_id)
            .finish_non_exhaustive()
    }
}
