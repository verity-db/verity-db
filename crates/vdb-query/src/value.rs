//! Typed SQL values.

use std::cmp::Ordering;
use std::fmt::{self, Display};

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use vdb_types::Timestamp;

use crate::error::{QueryError, Result};
use crate::schema::DataType;

/// A typed SQL value.
///
/// Represents values that can appear in query parameters, row data,
/// and comparison predicates.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    /// SQL NULL.
    #[default]
    Null,
    /// 64-bit signed integer.
    BigInt(i64),
    /// UTF-8 text string.
    Text(String),
    /// Boolean value.
    Boolean(bool),
    /// Timestamp (nanoseconds since epoch).
    Timestamp(Timestamp),
    /// Raw bytes (base64 encoded in JSON).
    #[serde(with = "bytes_base64")]
    Bytes(Bytes),
}

impl Value {
    /// Returns the data type of this value.
    ///
    /// Returns `None` for `Null` since NULL has no type.
    pub fn data_type(&self) -> Option<DataType> {
        match self {
            Value::Null => None,
            Value::BigInt(_) => Some(DataType::BigInt),
            Value::Text(_) => Some(DataType::Text),
            Value::Boolean(_) => Some(DataType::Boolean),
            Value::Timestamp(_) => Some(DataType::Timestamp),
            Value::Bytes(_) => Some(DataType::Bytes),
        }
    }

    /// Returns true if this value is NULL.
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// Returns the value as an i64, if it is a `BigInt`.
    pub fn as_bigint(&self) -> Option<i64> {
        match self {
            Value::BigInt(v) => Some(*v),
            _ => None,
        }
    }

    /// Returns the value as a string slice, if it is Text.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Value::Text(s) => Some(s),
            _ => None,
        }
    }

    /// Returns the value as a bool, if it is Boolean.
    pub fn as_boolean(&self) -> Option<bool> {
        match self {
            Value::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    /// Returns the value as a Timestamp, if it is Timestamp.
    pub fn as_timestamp(&self) -> Option<Timestamp> {
        match self {
            Value::Timestamp(ts) => Some(*ts),
            _ => None,
        }
    }

    /// Returns the value as bytes, if it is Bytes.
    pub fn as_bytes(&self) -> Option<&Bytes> {
        match self {
            Value::Bytes(b) => Some(b),
            _ => None,
        }
    }

    /// Compares two values for ordering.
    ///
    /// NULL values are considered less than all non-NULL values.
    /// Values of different types return None (incomparable).
    pub fn compare(&self, other: &Value) -> Option<Ordering> {
        match (self, other) {
            (Value::Null, Value::Null) => Some(Ordering::Equal),
            (Value::Null, _) => Some(Ordering::Less),
            (_, Value::Null) => Some(Ordering::Greater),
            (Value::BigInt(a), Value::BigInt(b)) => Some(a.cmp(b)),
            (Value::Text(a), Value::Text(b)) => Some(a.cmp(b)),
            (Value::Boolean(a), Value::Boolean(b)) => Some(a.cmp(b)),
            (Value::Timestamp(a), Value::Timestamp(b)) => Some(a.cmp(b)),
            (Value::Bytes(a), Value::Bytes(b)) => Some(a.as_ref().cmp(b.as_ref())),
            _ => None, // Different types are incomparable
        }
    }

    /// Checks if this value can be assigned to a column of the given type.
    pub fn is_compatible_with(&self, data_type: DataType) -> bool {
        match self {
            Value::Null => true, // NULL is compatible with any type
            Value::BigInt(_) => data_type == DataType::BigInt,
            Value::Text(_) => data_type == DataType::Text,
            Value::Boolean(_) => data_type == DataType::Boolean,
            Value::Timestamp(_) => data_type == DataType::Timestamp,
            Value::Bytes(_) => data_type == DataType::Bytes,
        }
    }

    /// Converts this value to JSON.
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Value::Null => serde_json::Value::Null,
            Value::BigInt(v) => serde_json::Value::Number((*v).into()),
            Value::Text(s) => serde_json::Value::String(s.clone()),
            Value::Boolean(b) => serde_json::Value::Bool(*b),
            Value::Timestamp(ts) => serde_json::Value::Number(ts.as_nanos().into()),
            Value::Bytes(b) => {
                use base64::Engine;
                let encoded = base64::engine::general_purpose::STANDARD.encode(b);
                serde_json::Value::String(encoded)
            }
        }
    }

    /// Parses a value from JSON with an expected data type.
    pub fn from_json(json: &serde_json::Value, data_type: DataType) -> Result<Self> {
        match (json, data_type) {
            (serde_json::Value::Null, _) => Ok(Value::Null),
            (serde_json::Value::Number(n), DataType::BigInt) => n
                .as_i64()
                .map(Value::BigInt)
                .ok_or_else(|| QueryError::TypeMismatch {
                    expected: "bigint".to_string(),
                    actual: format!("number {n}"),
                }),
            (serde_json::Value::String(s), DataType::Text) => Ok(Value::Text(s.clone())),
            (serde_json::Value::Bool(b), DataType::Boolean) => Ok(Value::Boolean(*b)),
            (serde_json::Value::Number(n), DataType::Timestamp) => n
                .as_u64()
                .map(|nanos| Value::Timestamp(Timestamp::from_nanos(nanos)))
                .ok_or_else(|| QueryError::TypeMismatch {
                    expected: "timestamp".to_string(),
                    actual: format!("number {n}"),
                }),
            (serde_json::Value::String(s), DataType::Bytes) => {
                use base64::Engine;
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(s)
                    .map_err(|e| QueryError::TypeMismatch {
                        expected: "base64 bytes".to_string(),
                        actual: e.to_string(),
                    })?;
                Ok(Value::Bytes(Bytes::from(decoded)))
            }
            (json, dt) => Err(QueryError::TypeMismatch {
                expected: format!("{dt:?}"),
                actual: format!("{json:?}"),
            }),
        }
    }
}

impl Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Null => write!(f, "NULL"),
            Value::BigInt(v) => write!(f, "{v}"),
            Value::Text(s) => write!(f, "'{s}'"),
            Value::Boolean(b) => write!(f, "{b}"),
            Value::Timestamp(ts) => write!(f, "{ts}"),
            Value::Bytes(b) => write!(f, "<{} bytes>", b.len()),
        }
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::BigInt(v)
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::Text(s)
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Text(s.to_string())
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Boolean(b)
    }
}

impl From<Timestamp> for Value {
    fn from(ts: Timestamp) -> Self {
        Value::Timestamp(ts)
    }
}

impl From<Bytes> for Value {
    fn from(b: Bytes) -> Self {
        Value::Bytes(b)
    }
}

/// Serde module for base64 encoding of bytes.
mod bytes_base64 {
    use base64::Engine;
    use bytes::Bytes;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &Bytes, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        serializer.serialize_str(&encoded)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Bytes, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&s)
            .map_err(serde::de::Error::custom)?;
        Ok(Bytes::from(decoded))
    }
}
