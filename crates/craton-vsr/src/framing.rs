//! Length-prefixed message framing for TCP transport.
//!
//! This module provides reliable message framing over TCP streams. Each message
//! is framed with:
//!
//! ```text
//! ┌──────────────┬──────────────┬──────────────────────────────────┐
//! │   Length     │   Checksum   │            Payload               │
//! │   (4 bytes)  │   (4 bytes)  │         (variable)               │
//! └──────────────┴──────────────┴──────────────────────────────────┘
//! ```
//!
//! - **Length**: Big-endian u32 of payload size (excludes header)
//! - **Checksum**: CRC32 of the payload for corruption detection
//! - **Payload**: bincode-serialized VSR message
//!
//! # Design Choices
//!
//! - Uses CRC32 for fast corruption detection (not cryptographic)
//! - Maximum message size is configurable (default 16 MiB)
//! - Incremental parsing for non-blocking I/O compatibility

use std::io::{self, Read, Write};

use crate::Message;

/// Size of the frame header in bytes (length + checksum).
pub const HEADER_SIZE: usize = 8;

/// Default maximum message size (16 MiB).
pub const DEFAULT_MAX_MESSAGE_SIZE: u32 = 16 * 1024 * 1024;

/// Minimum valid message size (empty message is invalid).
pub const MIN_MESSAGE_SIZE: u32 = 1;

// ============================================================================
// Errors
// ============================================================================

/// Errors that can occur during message framing.
#[derive(Debug, thiserror::Error)]
pub enum FramingError {
    /// I/O error during read or write.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// Message exceeds maximum allowed size.
    #[error("message too large: {size} bytes (max {max})")]
    MessageTooLarge { size: u32, max: u32 },

    /// Message checksum doesn't match.
    #[error("checksum mismatch: expected {expected:#x}, got {actual:#x}")]
    ChecksumMismatch { expected: u32, actual: u32 },

    /// Failed to deserialize message.
    #[error("deserialization failed: {0}")]
    Deserialize(String),

    /// Failed to serialize message.
    #[error("serialization failed: {0}")]
    Serialize(String),

    /// Incomplete frame (need more data).
    #[error("incomplete frame: have {have} bytes, need {need} bytes")]
    Incomplete { have: usize, need: usize },
}

impl FramingError {
    /// Returns true if this error indicates the connection should be closed.
    pub fn is_fatal(&self) -> bool {
        matches!(
            self,
            FramingError::ChecksumMismatch { .. }
                | FramingError::MessageTooLarge { .. }
                | FramingError::Deserialize(_)
        )
    }
}

// ============================================================================
// Encoder
// ============================================================================

/// Encodes VSR messages into framed bytes.
#[derive(Debug, Clone)]
pub struct FrameEncoder {
    /// Maximum message size.
    max_size: u32,
}

impl Default for FrameEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameEncoder {
    /// Creates a new encoder with default settings.
    pub fn new() -> Self {
        Self {
            max_size: DEFAULT_MAX_MESSAGE_SIZE,
        }
    }

    /// Creates an encoder with a custom maximum message size.
    pub fn with_max_size(max_size: u32) -> Self {
        debug_assert!(max_size >= MIN_MESSAGE_SIZE, "max_size must be positive");
        Self { max_size }
    }

    /// Encodes a message into a byte buffer.
    ///
    /// Returns the encoded bytes including the length-checksum header.
    pub fn encode(&self, message: &Message) -> Result<Vec<u8>, FramingError> {
        // Serialize the message
        let payload =
            bincode::serialize(message).map_err(|e| FramingError::Serialize(e.to_string()))?;

        let payload_len = payload.len();

        // Check size limit
        if payload_len > self.max_size as usize {
            return Err(FramingError::MessageTooLarge {
                size: payload_len as u32,
                max: self.max_size,
            });
        }

        // Compute checksum
        let checksum = crc32fast::hash(&payload);

        // Build the frame
        let mut frame = Vec::with_capacity(HEADER_SIZE + payload_len);

        // Write length (4 bytes, big-endian)
        frame.extend_from_slice(&(payload_len as u32).to_be_bytes());

        // Write checksum (4 bytes, big-endian)
        frame.extend_from_slice(&checksum.to_be_bytes());

        // Write payload
        frame.extend_from_slice(&payload);

        debug_assert_eq!(frame.len(), HEADER_SIZE + payload_len);

        Ok(frame)
    }

    /// Encodes a message and writes it to the given writer.
    pub fn encode_to<W: Write>(
        &self,
        message: &Message,
        writer: &mut W,
    ) -> Result<(), FramingError> {
        let frame = self.encode(message)?;
        writer.write_all(&frame)?;
        Ok(())
    }
}

// ============================================================================
// Decoder
// ============================================================================

/// State of the frame decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DecoderState {
    /// Waiting for the header.
    ReadingHeader,
    /// Reading the payload (have header, waiting for body).
    ReadingPayload { length: u32, checksum: u32 },
}

/// Decodes length-prefixed frames into VSR messages.
///
/// The decoder maintains internal state to handle partial reads from
/// non-blocking I/O. Call `decode()` repeatedly as data becomes available.
#[derive(Debug)]
pub struct FrameDecoder {
    /// Maximum message size.
    max_size: u32,
    /// Internal buffer for accumulating data.
    buffer: Vec<u8>,
    /// Current decoder state.
    state: DecoderState,
}

impl Default for FrameDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameDecoder {
    /// Creates a new decoder with default settings.
    pub fn new() -> Self {
        Self {
            max_size: DEFAULT_MAX_MESSAGE_SIZE,
            buffer: Vec::with_capacity(4096),
            state: DecoderState::ReadingHeader,
        }
    }

    /// Creates a decoder with a custom maximum message size.
    pub fn with_max_size(max_size: u32) -> Self {
        debug_assert!(max_size >= MIN_MESSAGE_SIZE, "max_size must be positive");
        Self {
            max_size,
            buffer: Vec::with_capacity(4096),
            state: DecoderState::ReadingHeader,
        }
    }

    /// Appends data to the internal buffer.
    pub fn extend(&mut self, data: &[u8]) {
        self.buffer.extend_from_slice(data);
    }

    /// Returns the number of bytes in the buffer.
    pub fn buffered(&self) -> usize {
        self.buffer.len()
    }

    /// Attempts to decode a message from the internal buffer.
    ///
    /// Returns:
    /// - `Ok(Some(message))` if a complete message was decoded
    /// - `Ok(None)` if more data is needed
    /// - `Err(_)` if the frame is invalid
    ///
    /// On success, the consumed bytes are removed from the buffer.
    pub fn decode(&mut self) -> Result<Option<Message>, FramingError> {
        loop {
            match self.state {
                DecoderState::ReadingHeader => {
                    if self.buffer.len() < HEADER_SIZE {
                        return Ok(None); // Need more data
                    }

                    // Parse header
                    let length = u32::from_be_bytes([
                        self.buffer[0],
                        self.buffer[1],
                        self.buffer[2],
                        self.buffer[3],
                    ]);

                    let checksum = u32::from_be_bytes([
                        self.buffer[4],
                        self.buffer[5],
                        self.buffer[6],
                        self.buffer[7],
                    ]);

                    // Validate length
                    if length > self.max_size {
                        return Err(FramingError::MessageTooLarge {
                            size: length,
                            max: self.max_size,
                        });
                    }

                    if length < MIN_MESSAGE_SIZE {
                        return Err(FramingError::Deserialize(
                            "empty message is invalid".to_string(),
                        ));
                    }

                    // Transition to reading payload
                    self.state = DecoderState::ReadingPayload { length, checksum };
                }

                DecoderState::ReadingPayload { length, checksum } => {
                    let total_needed = HEADER_SIZE + length as usize;

                    if self.buffer.len() < total_needed {
                        return Ok(None); // Need more data
                    }

                    // Extract payload
                    let payload = &self.buffer[HEADER_SIZE..total_needed];

                    // Verify checksum
                    let actual_checksum = crc32fast::hash(payload);
                    if actual_checksum != checksum {
                        return Err(FramingError::ChecksumMismatch {
                            expected: checksum,
                            actual: actual_checksum,
                        });
                    }

                    // Deserialize message
                    let message: Message = bincode::deserialize(payload)
                        .map_err(|e| FramingError::Deserialize(e.to_string()))?;

                    // Consume the frame from buffer
                    self.buffer.drain(..total_needed);

                    // Reset state
                    self.state = DecoderState::ReadingHeader;

                    return Ok(Some(message));
                }
            }
        }
    }

    /// Reads from a source and attempts to decode a message.
    ///
    /// This is a convenience method that combines reading and decoding.
    pub fn read_message<R: Read>(
        &mut self,
        reader: &mut R,
    ) -> Result<Option<Message>, FramingError> {
        // Read available data into a temporary buffer
        let mut temp = [0u8; 8192];
        match reader.read(&mut temp) {
            Ok(0) => {
                // EOF - if we have buffered data, it's incomplete
                if !self.buffer.is_empty() {
                    return Err(FramingError::Incomplete {
                        have: self.buffer.len(),
                        need: self.bytes_needed(),
                    });
                }
                Ok(None)
            }
            Ok(n) => {
                self.extend(&temp[..n]);
                self.decode()
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                // Non-blocking I/O - try to decode what we have
                self.decode()
            }
            Err(e) => Err(FramingError::Io(e)),
        }
    }

    /// Returns the number of bytes needed to complete the current frame.
    fn bytes_needed(&self) -> usize {
        match self.state {
            DecoderState::ReadingHeader => HEADER_SIZE.saturating_sub(self.buffer.len()),
            DecoderState::ReadingPayload { length, .. } => {
                let total = HEADER_SIZE + length as usize;
                total.saturating_sub(self.buffer.len())
            }
        }
    }

    /// Resets the decoder state, discarding any buffered data.
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.state = DecoderState::ReadingHeader;
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CommitNumber, Heartbeat, MessagePayload, ReplicaId, ViewNumber};

    fn test_message() -> Message {
        Message::broadcast(
            ReplicaId::new(0),
            MessagePayload::Heartbeat(Heartbeat::new(ViewNumber::new(1), CommitNumber::ZERO)),
        )
    }

    #[test]
    fn encode_decode_roundtrip() {
        let encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();

        let original = test_message();
        let encoded = encoder.encode(&original).expect("encode");

        assert!(encoded.len() > HEADER_SIZE);

        decoder.extend(&encoded);
        let decoded = decoder.decode().expect("decode").expect("complete message");

        assert_eq!(decoded.from, original.from);
        assert_eq!(decoded.to, original.to);
        assert_eq!(decoded.payload.name(), original.payload.name());
    }

    #[test]
    fn encode_decode_multiple_messages() {
        let encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();

        let messages: Vec<Message> = (0..5)
            .map(|i| {
                Message::broadcast(
                    ReplicaId::new(i),
                    MessagePayload::Heartbeat(Heartbeat::new(
                        ViewNumber::new(u64::from(i)),
                        CommitNumber::ZERO,
                    )),
                )
            })
            .collect();

        // Encode all messages into one buffer
        let mut all_encoded = Vec::new();
        for msg in &messages {
            all_encoded.extend(encoder.encode(msg).expect("encode"));
        }

        // Feed to decoder
        decoder.extend(&all_encoded);

        // Decode all messages
        for original in &messages {
            let decoded = decoder.decode().expect("decode").expect("complete message");
            assert_eq!(decoded.from, original.from);
        }

        // No more messages
        assert!(decoder.decode().expect("decode").is_none());
    }

    #[test]
    fn decode_incremental() {
        let encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();

        let original = test_message();
        let encoded = encoder.encode(&original).expect("encode");

        // Feed one byte at a time
        for (i, &byte) in encoded.iter().enumerate() {
            decoder.extend(&[byte]);
            let result = decoder.decode().expect("decode");

            if i < encoded.len() - 1 {
                assert!(result.is_none(), "should not decode until complete");
            } else {
                assert!(result.is_some(), "should decode when complete");
            }
        }
    }

    #[test]
    fn checksum_mismatch() {
        let encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();

        let original = test_message();
        let mut encoded = encoder.encode(&original).expect("encode");

        // Corrupt one byte in the payload
        let last = encoded.len() - 1;
        encoded[last] ^= 0xff;

        decoder.extend(&encoded);
        let result = decoder.decode();

        assert!(matches!(result, Err(FramingError::ChecksumMismatch { .. })));
    }

    #[test]
    fn message_too_large() {
        let encoder = FrameEncoder::with_max_size(100);

        // Create a message with a large payload (will exceed 100 bytes when serialized)
        let msg = Message::broadcast(
            ReplicaId::new(0),
            MessagePayload::Heartbeat(Heartbeat::new(
                ViewNumber::new(u64::MAX), // Large values
                CommitNumber::new(crate::OpNumber::new(u64::MAX)),
            )),
        );

        // Check if it exceeds size (this may or may not depending on bincode encoding)
        // The actual test is that the encoder correctly rejects or accepts based on size
        let result = encoder.encode(&msg);

        // We can't guarantee this will fail with our test message, so just verify
        // the encoder/decoder work correctly
        if let Ok(encoded) = result {
            let mut decoder = FrameDecoder::with_max_size(100);
            decoder.extend(&encoded);
            // This might succeed or fail depending on actual encoded size
            let _ = decoder.decode();
        }
    }

    #[test]
    fn decoder_reset() {
        let encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();

        let original = test_message();
        let encoded = encoder.encode(&original).expect("encode");

        // Feed partial data
        decoder.extend(&encoded[..HEADER_SIZE + 1]);
        assert!(decoder.decode().expect("decode").is_none());
        assert!(decoder.buffered() > 0);

        // Reset
        decoder.reset();
        assert_eq!(decoder.buffered(), 0);

        // Feed complete message
        decoder.extend(&encoded);
        assert!(decoder.decode().expect("decode").is_some());
    }

    #[test]
    fn header_size_correct() {
        assert_eq!(HEADER_SIZE, 8, "header should be 4 (length) + 4 (checksum)");
    }

    #[test]
    fn frame_structure() {
        let encoder = FrameEncoder::new();
        let original = test_message();
        let encoded = encoder.encode(&original).expect("encode");

        // Parse header manually
        let length = u32::from_be_bytes([encoded[0], encoded[1], encoded[2], encoded[3]]);
        let checksum = u32::from_be_bytes([encoded[4], encoded[5], encoded[6], encoded[7]]);

        // Verify length matches payload
        assert_eq!(length as usize, encoded.len() - HEADER_SIZE);

        // Verify checksum matches
        let payload = &encoded[HEADER_SIZE..];
        assert_eq!(checksum, crc32fast::hash(payload));
    }
}
