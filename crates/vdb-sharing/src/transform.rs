//! Data transformation operations for export.
//!
//! Provides anonymization, pseudonymization, and other transformations
//! for secure data sharing.

use serde_json::Value as JsonValue;
use vdb_crypto::{HashPurpose, hash_with_purpose};

use crate::error::{SharingError, SharingResult};
use crate::scope::{TransformationRule, TransformationType};

/// Applies transformations to a JSON record.
pub fn transform_record(
    record: &mut serde_json::Map<String, JsonValue>,
    rules: &[TransformationRule],
) -> SharingResult<()> {
    let fields: Vec<String> = record.keys().cloned().collect();

    for field in fields {
        for rule in rules {
            if rule.matches_field(&field) {
                if let Some(value) = record.get(&field).cloned() {
                    let transformed = apply_transformation(&value, &rule.transformation)?;
                    record.insert(field.clone(), transformed);
                }
            }
        }
    }

    Ok(())
}

/// Applies a transformation to a JSON value.
pub fn apply_transformation(
    value: &JsonValue,
    transformation: &TransformationType,
) -> SharingResult<JsonValue> {
    match transformation {
        TransformationType::Redact => Ok(JsonValue::Null),

        TransformationType::Mask => {
            let masked = match value {
                JsonValue::String(s) => "*".repeat(s.len()),
                JsonValue::Number(n) => "*".repeat(n.to_string().len()),
                _ => return Ok(JsonValue::String("****".to_string())),
            };
            Ok(JsonValue::String(masked))
        }

        TransformationType::PartialMask {
            show_start,
            show_end,
        } => {
            let s = match value {
                JsonValue::String(s) => s.clone(),
                JsonValue::Number(n) => n.to_string(),
                _ => return Ok(value.clone()),
            };

            let masked = partial_mask(&s, *show_start, *show_end);
            Ok(JsonValue::String(masked))
        }

        TransformationType::Pseudonymize => {
            // Create a deterministic pseudonym from the value
            let input = match value {
                JsonValue::String(s) => s.clone(),
                JsonValue::Number(n) => n.to_string(),
                JsonValue::Bool(b) => b.to_string(),
                _ => serde_json::to_string(value).unwrap_or_default(),
            };

            let (_, hash) = hash_with_purpose(HashPurpose::Internal, input.as_bytes());
            let hex = bytes_to_hex(&hash[..8]);
            let pseudonym = format!("PSEUDO-{hex}");
            Ok(JsonValue::String(pseudonym))
        }

        TransformationType::Generalize { bucket_size } => {
            match value {
                JsonValue::Number(n) => {
                    if let Some(i) = n.as_i64() {
                        let bucket = bucket_size.unwrap_or(10);
                        // Handle negative values by taking absolute value
                        #[allow(clippy::cast_sign_loss)]
                        let abs_i = i.unsigned_abs();
                        let lower = (abs_i / bucket) * bucket;
                        let upper = lower + bucket - 1;
                        let sign = if i < 0 { "-" } else { "" };
                        Ok(JsonValue::String(format!("{sign}{lower}-{sign}{upper}")))
                    } else if let Some(f) = n.as_f64() {
                        // For f64, precision loss is acceptable for generalization
                        #[allow(clippy::cast_precision_loss)]
                        let bucket = bucket_size.unwrap_or(10) as f64;
                        let lower = (f / bucket).floor() * bucket;
                        let upper = lower + bucket;
                        Ok(JsonValue::String(format!("{lower:.0}-{upper:.0}")))
                    } else {
                        Ok(value.clone())
                    }
                }
                _ => Ok(value.clone()),
            }
        }

        TransformationType::Hash => {
            let input = match value {
                JsonValue::String(s) => s.clone(),
                JsonValue::Number(n) => n.to_string(),
                JsonValue::Bool(b) => b.to_string(),
                _ => serde_json::to_string(value).unwrap_or_default(),
            };

            let (_, hash) = hash_with_purpose(HashPurpose::Compliance, input.as_bytes());
            Ok(JsonValue::String(bytes_to_hex(&hash)))
        }

        TransformationType::Encrypt => {
            // Encryption requires a key, which should be handled at a higher level
            Err(SharingError::InvalidTransformation(
                "Encrypt transformation requires key context".to_string(),
            ))
        }
    }
}

/// Converts bytes to a hex string.
fn bytes_to_hex(bytes: &[u8]) -> String {
    const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(HEX_CHARS[(byte >> 4) as usize] as char);
        result.push(HEX_CHARS[(byte & 0xf) as usize] as char);
    }
    result
}

/// Partially masks a string, showing only start and end characters.
fn partial_mask(s: &str, show_start: usize, show_end: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();

    if len <= show_start + show_end {
        return "*".repeat(len);
    }

    let start: String = chars[..show_start].iter().collect();
    let end: String = chars[len - show_end..].iter().collect();
    let middle_len = len - show_start - show_end;

    format!("{}{}{}", start, "*".repeat(middle_len), end)
}

/// Transforms multiple fields according to the rules.
pub struct Transformer {
    rules: Vec<TransformationRule>,
}

impl Transformer {
    /// Creates a new transformer with the given rules.
    pub fn new(rules: Vec<TransformationRule>) -> Self {
        Self { rules }
    }

    /// Transforms a JSON object.
    pub fn transform(&self, record: &mut serde_json::Map<String, JsonValue>) -> SharingResult<()> {
        transform_record(record, &self.rules)
    }

    /// Transforms a list of JSON objects.
    pub fn transform_batch(
        &self,
        records: &mut [serde_json::Map<String, JsonValue>],
    ) -> SharingResult<()> {
        for record in records {
            self.transform(record)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_redact() {
        let value = json!("secret");
        let result = apply_transformation(&value, &TransformationType::Redact).unwrap();
        assert_eq!(result, JsonValue::Null);
    }

    #[test]
    fn test_mask() {
        let value = json!("password123");
        let result = apply_transformation(&value, &TransformationType::Mask).unwrap();
        assert_eq!(result, json!("***********"));

        let num_value = json!(12345);
        let result = apply_transformation(&num_value, &TransformationType::Mask).unwrap();
        assert_eq!(result, json!("*****"));
    }

    #[test]
    fn test_partial_mask() {
        let value = json!("1234567890");
        let result = apply_transformation(
            &value,
            &TransformationType::PartialMask {
                show_start: 0,
                show_end: 4,
            },
        )
        .unwrap();
        assert_eq!(result, json!("******7890"));

        let card = json!("4111111111111111");
        let result = apply_transformation(
            &card,
            &TransformationType::PartialMask {
                show_start: 4,
                show_end: 4,
            },
        )
        .unwrap();
        assert_eq!(result, json!("4111********1111"));
    }

    #[test]
    fn test_pseudonymize() {
        let value = json!("patient-123");
        let result = apply_transformation(&value, &TransformationType::Pseudonymize).unwrap();

        // Should be deterministic
        let result2 = apply_transformation(&value, &TransformationType::Pseudonymize).unwrap();
        assert_eq!(result, result2);

        // Should start with PSEUDO-
        if let JsonValue::String(s) = result {
            assert!(s.starts_with("PSEUDO-"));
            assert_eq!(s.len(), 23); // "PSEUDO-" + 16 hex chars
        } else {
            panic!("Expected string");
        }
    }

    #[test]
    fn test_generalize() {
        let value = json!(42);
        let result = apply_transformation(
            &value,
            &TransformationType::Generalize {
                bucket_size: Some(10),
            },
        )
        .unwrap();
        assert_eq!(result, json!("40-49"));

        let value2 = json!(100);
        let result2 = apply_transformation(
            &value2,
            &TransformationType::Generalize {
                bucket_size: Some(100),
            },
        )
        .unwrap();
        assert_eq!(result2, json!("100-199"));
    }

    #[test]
    fn test_hash() {
        let value = json!("sensitive-data");
        let result = apply_transformation(&value, &TransformationType::Hash).unwrap();

        if let JsonValue::String(s) = result {
            assert_eq!(s.len(), 64); // SHA-256 hex is 64 chars
        } else {
            panic!("Expected string");
        }
    }

    #[test]
    fn test_transform_record() {
        let mut record = serde_json::Map::new();
        record.insert("name".to_string(), json!("John Doe"));
        record.insert("ssn".to_string(), json!("123-45-6789"));
        record.insert("email".to_string(), json!("john@example.com"));

        let rules = vec![
            TransformationRule::new("ssn", TransformationType::Redact),
            TransformationRule::new("email", TransformationType::Hash),
        ];

        transform_record(&mut record, &rules).unwrap();

        assert_eq!(record.get("name"), Some(&json!("John Doe")));
        assert_eq!(record.get("ssn"), Some(&JsonValue::Null));
        assert!(record.get("email").unwrap().as_str().unwrap().len() == 64);
    }

    #[test]
    fn test_partial_mask_edge_cases() {
        // String shorter than show_start + show_end
        assert_eq!(partial_mask("abc", 2, 2), "***");
        assert_eq!(partial_mask("ab", 2, 2), "**");

        // Exact length
        assert_eq!(partial_mask("abcd", 2, 2), "****");

        // Normal case
        assert_eq!(partial_mask("abcdefgh", 2, 2), "ab****gh");
    }
}
