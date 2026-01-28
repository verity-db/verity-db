//! Lexicographic key encoding for B+tree lookups.
//!
//! This module provides encoding functions that preserve ordering when
//! the encoded bytes are compared lexicographically. This enables efficient
//! range scans on the B+tree store.
//!
//! # Encoding Strategies
//!
//! - **`BigInt`**: Sign-flip encoding (XOR with 0x80 on high byte)
//! - **Text**: UTF-8 bytes as-is (already lexicographic)
//! - **Bytes**: Raw bytes as-is
//! - **Timestamp**: Big-endian u64 (naturally lexicographic)
//! - **Boolean**: 0x00 for false, 0x01 for true

use bytes::Bytes;
use craton_store::Key;
use craton_types::Timestamp;

use crate::value::Value;

/// Encodes a `BigInt` for lexicographic ordering.
///
/// Uses sign-flip encoding: XOR the high byte with 0x80 so that
/// negative numbers sort before positive numbers in byte order.
///
/// ```text
/// i64::MIN (-9223372036854775808) -> 0x00_00_00_00_00_00_00_00
/// -1                              -> 0x7F_FF_FF_FF_FF_FF_FF_FF
///  0                              -> 0x80_00_00_00_00_00_00_00
///  1                              -> 0x80_00_00_00_00_00_00_01
/// i64::MAX (9223372036854775807)  -> 0xFF_FF_FF_FF_FF_FF_FF_FF
/// ```
#[allow(clippy::cast_sign_loss)]
pub fn encode_bigint(value: i64) -> [u8; 8] {
    // Convert to big-endian, then flip the sign bit
    // The cast is intentional for sign-flip encoding.
    let unsigned = (value as u64) ^ (1u64 << 63);
    unsigned.to_be_bytes()
}

/// Decodes a `BigInt` from sign-flip encoding.
#[allow(dead_code)]
pub fn decode_bigint(bytes: [u8; 8]) -> i64 {
    let unsigned = u64::from_be_bytes(bytes);
    (unsigned ^ (1u64 << 63)) as i64
}

/// Encodes a Timestamp for lexicographic ordering.
///
/// Timestamps are u64 nanoseconds, which are naturally ordered
/// when stored as big-endian bytes.
pub fn encode_timestamp(ts: Timestamp) -> [u8; 8] {
    ts.as_nanos().to_be_bytes()
}

/// Decodes a Timestamp from big-endian encoding.
#[allow(dead_code)]
pub fn decode_timestamp(bytes: [u8; 8]) -> Timestamp {
    Timestamp::from_nanos(u64::from_be_bytes(bytes))
}

/// Encodes a boolean for lexicographic ordering.
pub fn encode_boolean(value: bool) -> [u8; 1] {
    [u8::from(value)]
}

/// Decodes a boolean from its encoded form.
#[allow(dead_code)]
pub fn decode_boolean(byte: u8) -> bool {
    byte != 0
}

/// Encodes a composite key from multiple values.
///
/// Each value is length-prefixed to enable unambiguous decoding
/// and to handle variable-length types correctly.
///
/// # Encoding Format
///
/// For each value:
/// - 1 byte: Type tag (0=Null, 1=BigInt, 2=Text, 3=Boolean, 4=Timestamp, 5=Bytes)
/// - Variable: Encoded value
///
/// For variable-length types (Text, Bytes):
/// - 4 bytes: Length (big-endian u32)
/// - N bytes: Data
pub fn encode_key(values: &[Value]) -> Key {
    let mut buf = Vec::with_capacity(64);

    for value in values {
        match value {
            Value::Null => {
                buf.push(0x00); // Type tag for NULL
            }
            Value::BigInt(v) => {
                buf.push(0x01); // Type tag for BigInt
                buf.extend_from_slice(&encode_bigint(*v));
            }
            Value::Text(s) => {
                buf.push(0x02); // Type tag for Text
                let bytes = s.as_bytes();
                buf.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
                buf.extend_from_slice(bytes);
            }
            Value::Boolean(b) => {
                buf.push(0x03); // Type tag for Boolean
                buf.extend_from_slice(&encode_boolean(*b));
            }
            Value::Timestamp(ts) => {
                buf.push(0x04); // Type tag for Timestamp
                buf.extend_from_slice(&encode_timestamp(*ts));
            }
            Value::Bytes(b) => {
                buf.push(0x05); // Type tag for Bytes
                buf.extend_from_slice(&(b.len() as u32).to_be_bytes());
                buf.extend_from_slice(b);
            }
        }
    }

    Key::from(buf)
}

/// Decodes a composite key back into values.
///
/// # Panics
///
/// Panics if the encoded data is malformed.
#[allow(dead_code)]
pub fn decode_key(key: &Key) -> Vec<Value> {
    let bytes = key.as_bytes();
    let mut values = Vec::new();
    let mut pos = 0;

    while pos < bytes.len() {
        let tag = bytes[pos];
        pos += 1;

        let value = match tag {
            0x00 => Value::Null,
            0x01 => {
                // BigInt: 8 bytes
                debug_assert!(
                    pos + 8 <= bytes.len(),
                    "insufficient bytes for BigInt at position {pos}"
                );
                let arr: [u8; 8] = bytes[pos..pos + 8]
                    .try_into()
                    .expect("BigInt decode failed");
                pos += 8;
                Value::BigInt(decode_bigint(arr))
            }
            0x02 => {
                // Text: 4-byte length + data
                debug_assert!(
                    pos + 4 <= bytes.len(),
                    "insufficient bytes for Text length at position {pos}"
                );
                let len = u32::from_be_bytes(
                    bytes[pos..pos + 4]
                        .try_into()
                        .expect("Text length decode failed"),
                ) as usize;
                pos += 4;
                debug_assert!(
                    pos + len <= bytes.len(),
                    "insufficient bytes for Text data at position {pos}"
                );
                let s = std::str::from_utf8(&bytes[pos..pos + len])
                    .expect("Text decode failed: invalid UTF-8");
                pos += len;
                Value::Text(s.to_string())
            }
            0x03 => {
                // Boolean: 1 byte
                debug_assert!(
                    pos < bytes.len(),
                    "insufficient bytes for Boolean at position {pos}"
                );
                let b = decode_boolean(bytes[pos]);
                pos += 1;
                Value::Boolean(b)
            }
            0x04 => {
                // Timestamp: 8 bytes
                debug_assert!(
                    pos + 8 <= bytes.len(),
                    "insufficient bytes for Timestamp at position {pos}"
                );
                let arr: [u8; 8] = bytes[pos..pos + 8]
                    .try_into()
                    .expect("Timestamp decode failed");
                pos += 8;
                Value::Timestamp(decode_timestamp(arr))
            }
            0x05 => {
                // Bytes: 4-byte length + data
                debug_assert!(
                    pos + 4 <= bytes.len(),
                    "insufficient bytes for Bytes length at position {pos}"
                );
                let len = u32::from_be_bytes(
                    bytes[pos..pos + 4]
                        .try_into()
                        .expect("Bytes length decode failed"),
                ) as usize;
                pos += 4;
                debug_assert!(
                    pos + len <= bytes.len(),
                    "insufficient bytes for Bytes data at position {pos}"
                );
                let data = Bytes::copy_from_slice(&bytes[pos..pos + len]);
                pos += len;
                Value::Bytes(data)
            }
            _ => panic!("unknown type tag {tag} at position {}", pos - 1),
        };

        values.push(value);
    }

    values
}

/// Creates a key that represents the minimum value for a type.
///
/// Useful for constructing range scan bounds.
#[allow(dead_code)]
pub fn min_key_for_type(count: usize) -> Key {
    let values: Vec<Value> = (0..count).map(|_| Value::Null).collect();
    encode_key(&values)
}

/// Creates a key that is greater than any key starting with the given prefix.
///
/// Used for exclusive upper bounds in range scans.
pub fn successor_key(key: &Key) -> Key {
    let bytes = key.as_bytes();
    let mut result = bytes.to_vec();

    // Increment the last byte, carrying as needed
    for i in (0..result.len()).rev() {
        if result[i] < 0xFF {
            result[i] += 1;
            return Key::from(result);
        }
        result[i] = 0x00;
    }

    // All bytes were 0xFF, append 0x00 to make it larger
    result.push(0x00);
    Key::from(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bigint_encoding_preserves_order() {
        let values = [
            i64::MIN,
            i64::MIN + 1,
            -1000,
            -1,
            0,
            1,
            1000,
            i64::MAX - 1,
            i64::MAX,
        ];

        let encoded: Vec<_> = values.iter().map(|&v| encode_bigint(v)).collect();
        let mut sorted = encoded.clone();
        sorted.sort();

        assert_eq!(encoded, sorted, "BigInt encoding should preserve ordering");

        // Verify decode round-trips
        for &v in &values {
            assert_eq!(decode_bigint(encode_bigint(v)), v);
        }
    }

    #[test]
    fn test_timestamp_encoding_preserves_order() {
        let values = [0u64, 1, 1000, u64::MAX / 2, u64::MAX];

        let encoded: Vec<_> = values
            .iter()
            .map(|&v| encode_timestamp(Timestamp::from_nanos(v)))
            .collect();
        let mut sorted = encoded.clone();
        sorted.sort();

        assert_eq!(
            encoded, sorted,
            "Timestamp encoding should preserve ordering"
        );

        // Verify decode round-trips
        for &v in &values {
            let ts = Timestamp::from_nanos(v);
            assert_eq!(decode_timestamp(encode_timestamp(ts)), ts);
        }
    }

    #[test]
    fn test_composite_key_round_trip() {
        let values = vec![
            Value::BigInt(42),
            Value::Text("hello".to_string()),
            Value::Boolean(true),
            Value::Timestamp(Timestamp::from_nanos(12345)),
            Value::Bytes(Bytes::from_static(b"data")),
        ];

        let key = encode_key(&values);
        let decoded = decode_key(&key);

        assert_eq!(values, decoded);
    }

    #[test]
    fn test_composite_key_ordering() {
        // Keys with same first value, different second value
        let key1 = encode_key(&[Value::BigInt(1), Value::BigInt(1)]);
        let key2 = encode_key(&[Value::BigInt(1), Value::BigInt(2)]);
        let key3 = encode_key(&[Value::BigInt(2), Value::BigInt(1)]);

        assert!(key1 < key2, "key1 should be less than key2");
        assert!(key2 < key3, "key2 should be less than key3");
    }

    #[test]
    fn test_successor_key() {
        let key = encode_key(&[Value::BigInt(42)]);
        let succ = successor_key(&key);

        assert!(key < succ, "successor should be greater");
    }

    #[test]
    fn test_null_handling() {
        let key = encode_key(&[Value::Null]);
        let decoded = decode_key(&key);
        assert_eq!(decoded, vec![Value::Null]);
    }
}
