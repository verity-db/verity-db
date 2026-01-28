//! Frame encoding and decoding for the wire protocol.
//!
//! A frame consists of a fixed-size header followed by a variable-size payload.

use bytes::{Buf, BufMut, Bytes, BytesMut};
use crc32fast::Hasher;

use crate::error::{WireError, WireResult};

/// Protocol magic bytes: "VDB " in big-endian.
pub const MAGIC: u32 = 0x5644_4220;

/// Current protocol version.
pub const PROTOCOL_VERSION: u16 = 1;

/// Frame header size in bytes (magic + version + length + checksum).
pub const FRAME_HEADER_SIZE: usize = 14;

/// Maximum payload size (16 MiB).
pub const MAX_PAYLOAD_SIZE: u32 = 16 * 1024 * 1024;

/// Frame header containing metadata about the payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    /// Protocol magic bytes.
    pub magic: u32,
    /// Protocol version.
    pub version: u16,
    /// Payload length in bytes.
    pub length: u32,
    /// CRC32 checksum of the payload.
    pub checksum: u32,
}

impl FrameHeader {
    /// Creates a new frame header for the given payload.
    pub fn new(payload: &[u8]) -> Self {
        let checksum = compute_checksum(payload);
        Self {
            magic: MAGIC,
            version: PROTOCOL_VERSION,
            length: payload.len() as u32,
            checksum,
        }
    }

    /// Encodes the header to bytes.
    pub fn encode(&self, buf: &mut BytesMut) {
        buf.put_u32(self.magic);
        buf.put_u16(self.version);
        buf.put_u32(self.length);
        buf.put_u32(self.checksum);
    }

    /// Decodes a header from bytes.
    ///
    /// Returns `None` if there aren't enough bytes.
    pub fn decode(buf: &mut impl Buf) -> Option<Self> {
        if buf.remaining() < FRAME_HEADER_SIZE {
            return None;
        }

        Some(Self {
            magic: buf.get_u32(),
            version: buf.get_u16(),
            length: buf.get_u32(),
            checksum: buf.get_u32(),
        })
    }

    /// Validates the header.
    pub fn validate(&self) -> WireResult<()> {
        if self.magic != MAGIC {
            return Err(WireError::InvalidMagic(self.magic));
        }

        if self.version != PROTOCOL_VERSION {
            return Err(WireError::UnsupportedVersion(self.version));
        }

        if self.length > MAX_PAYLOAD_SIZE {
            return Err(WireError::PayloadTooLarge {
                size: self.length,
                max: MAX_PAYLOAD_SIZE,
            });
        }

        Ok(())
    }
}

/// A complete frame with header and payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Frame header.
    pub header: FrameHeader,
    /// Payload bytes.
    pub payload: Bytes,
}

impl Frame {
    /// Creates a new frame from a payload.
    pub fn new(payload: Bytes) -> Self {
        let header = FrameHeader::new(&payload);
        Self { header, payload }
    }

    /// Encodes the frame to a byte buffer.
    pub fn encode(&self, buf: &mut BytesMut) {
        self.header.encode(buf);
        buf.put_slice(&self.payload);
    }

    /// Encodes the frame to a new byte buffer.
    pub fn encode_to_bytes(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(FRAME_HEADER_SIZE + self.payload.len());
        self.encode(&mut buf);
        buf.freeze()
    }

    /// Attempts to decode a frame from a byte buffer.
    ///
    /// Returns `Ok(Some(frame))` if a complete frame was decoded.
    /// Returns `Ok(None)` if more bytes are needed.
    /// Returns `Err` if the frame is invalid.
    ///
    /// On success, the consumed bytes are removed from the buffer.
    pub fn decode(buf: &mut BytesMut) -> WireResult<Option<Self>> {
        // Check if we have enough bytes for the header
        if buf.len() < FRAME_HEADER_SIZE {
            return Ok(None);
        }

        // Peek at the header without consuming
        let header = {
            let mut peek = buf.as_ref();
            FrameHeader::decode(&mut peek).expect("checked length above")
        };

        // Validate header
        header.validate()?;

        // Check if we have the complete payload
        let total_size = FRAME_HEADER_SIZE + header.length as usize;
        if buf.len() < total_size {
            return Ok(None);
        }

        // Consume header
        buf.advance(FRAME_HEADER_SIZE);

        // Extract payload
        let payload = buf.split_to(header.length as usize).freeze();

        // Verify checksum
        let actual_checksum = compute_checksum(&payload);
        if actual_checksum != header.checksum {
            return Err(WireError::ChecksumMismatch {
                expected: header.checksum,
                actual: actual_checksum,
            });
        }

        Ok(Some(Self { header, payload }))
    }

    /// Returns the total size of the frame in bytes.
    pub fn total_size(&self) -> usize {
        FRAME_HEADER_SIZE + self.payload.len()
    }
}

/// Computes CRC32 checksum of data.
fn compute_checksum(data: &[u8]) -> u32 {
    let mut hasher = Hasher::new();
    hasher.update(data);
    hasher.finalize()
}

#[cfg(test)]
mod frame_tests {
    use super::*;

    #[test]
    fn test_frame_roundtrip() {
        let payload = Bytes::from("hello, world!");
        let frame = Frame::new(payload.clone());

        // Encode
        let encoded = frame.encode_to_bytes();
        assert_eq!(encoded.len(), FRAME_HEADER_SIZE + payload.len());

        // Decode
        let mut buf = BytesMut::from(&encoded[..]);
        let decoded = Frame::decode(&mut buf).unwrap().unwrap();

        assert_eq!(decoded.payload, payload);
        assert!(buf.is_empty());
    }

    #[test]
    fn test_incomplete_header() {
        let mut buf = BytesMut::from(&[0u8; 5][..]);
        assert!(Frame::decode(&mut buf).unwrap().is_none());
    }

    #[test]
    fn test_incomplete_payload() {
        let payload = Bytes::from("test");
        let frame = Frame::new(payload);
        let encoded = frame.encode_to_bytes();

        // Only provide part of the frame
        let mut buf = BytesMut::from(&encoded[..FRAME_HEADER_SIZE + 2]);
        assert!(Frame::decode(&mut buf).unwrap().is_none());
    }

    #[test]
    fn test_invalid_magic() {
        let mut buf = BytesMut::new();
        buf.put_u32(0xDEADBEEF); // Wrong magic
        buf.put_u16(PROTOCOL_VERSION);
        buf.put_u32(4);
        buf.put_u32(0);
        buf.put_slice(b"test");

        let result = Frame::decode(&mut buf);
        assert!(matches!(result, Err(WireError::InvalidMagic(0xDEADBEEF))));
    }

    #[test]
    fn test_checksum_mismatch() {
        let mut buf = BytesMut::new();
        buf.put_u32(MAGIC);
        buf.put_u16(PROTOCOL_VERSION);
        buf.put_u32(4);
        buf.put_u32(0xBADBAD); // Wrong checksum
        buf.put_slice(b"test");

        let result = Frame::decode(&mut buf);
        assert!(matches!(result, Err(WireError::ChecksumMismatch { .. })));
    }

    #[test]
    fn test_header_constants() {
        assert_eq!(MAGIC, 0x5644_4220);
        assert_eq!(FRAME_HEADER_SIZE, 14);
        assert_eq!(MAX_PAYLOAD_SIZE, 16 * 1024 * 1024);
    }
}
