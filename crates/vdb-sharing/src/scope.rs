//! Export scope definitions for data sharing.
//!
//! Scopes define what data can be accessed and how it can be transformed.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use vdb_types::{Offset, StreamId};

/// Defines the scope of data that can be exported.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportScope {
    /// Allowed streams (empty = all streams).
    pub streams: HashSet<StreamId>,
    /// Allowed tables (empty = all tables).
    pub tables: HashSet<String>,
    /// Allowed fields (empty = all fields).
    pub fields: HashSet<String>,
    /// Excluded fields (applied after allowed fields).
    pub excluded_fields: HashSet<String>,
    /// Time range filter.
    pub time_range: Option<TimeRange>,
    /// Log position range filter.
    pub position_range: Option<PositionRange>,
    /// Maximum number of rows per query.
    pub max_rows: Option<u64>,
    /// Required transformations.
    pub transformations: Vec<TransformationRule>,
}

impl ExportScope {
    /// Creates a scope with full access.
    pub fn full() -> Self {
        Self {
            streams: HashSet::new(),
            tables: HashSet::new(),
            fields: HashSet::new(),
            excluded_fields: HashSet::new(),
            time_range: None,
            position_range: None,
            max_rows: None,
            transformations: Vec::new(),
        }
    }

    /// Creates a scope limited to specific tables.
    pub fn tables(tables: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            tables: tables.into_iter().map(Into::into).collect(),
            ..Self::full()
        }
    }

    /// Creates a scope limited to specific fields.
    pub fn fields(fields: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            fields: fields.into_iter().map(Into::into).collect(),
            ..Self::full()
        }
    }

    /// Adds allowed fields to the scope (chainable).
    pub fn with_fields(mut self, fields: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.fields = fields.into_iter().map(Into::into).collect();
        self
    }

    /// Adds a time range constraint.
    pub fn with_time_range(mut self, start: DateTime<Utc>, end: DateTime<Utc>) -> Self {
        self.time_range = Some(TimeRange { start, end });
        self
    }

    /// Adds a position range constraint.
    pub fn with_position_range(mut self, start: Offset, end: Offset) -> Self {
        self.position_range = Some(PositionRange { start, end });
        self
    }

    /// Adds a max rows constraint.
    pub fn with_max_rows(mut self, max: u64) -> Self {
        self.max_rows = Some(max);
        self
    }

    /// Adds a transformation rule.
    pub fn with_transformation(mut self, rule: TransformationRule) -> Self {
        self.transformations.push(rule);
        self
    }

    /// Excludes specific fields from export.
    pub fn exclude_fields(mut self, fields: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.excluded_fields = fields.into_iter().map(Into::into).collect();
        self
    }

    /// Checks if a stream is allowed by this scope.
    pub fn allows_stream(&self, stream_id: StreamId) -> bool {
        self.streams.is_empty() || self.streams.contains(&stream_id)
    }

    /// Checks if a table is allowed by this scope.
    pub fn allows_table(&self, table: &str) -> bool {
        self.tables.is_empty() || self.tables.contains(table)
    }

    /// Checks if a field is allowed by this scope.
    pub fn allows_field(&self, field: &str) -> bool {
        let allowed = self.fields.is_empty() || self.fields.contains(field);
        let not_excluded = !self.excluded_fields.contains(field);
        allowed && not_excluded
    }

    /// Checks if a timestamp is within the allowed range.
    pub fn allows_time(&self, time: DateTime<Utc>) -> bool {
        match &self.time_range {
            Some(range) => time >= range.start && time <= range.end,
            None => true,
        }
    }

    /// Checks if a position is within the allowed range.
    pub fn allows_position(&self, position: Offset) -> bool {
        match &self.position_range {
            Some(range) => position >= range.start && position <= range.end,
            None => true,
        }
    }

    /// Filters a list of fields to only those allowed.
    pub fn filter_fields(&self, fields: &[String]) -> Vec<String> {
        fields
            .iter()
            .filter(|f| self.allows_field(f))
            .cloned()
            .collect()
    }
}

/// Time range for filtering exports.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeRange {
    /// Start time (inclusive).
    pub start: DateTime<Utc>,
    /// End time (inclusive).
    pub end: DateTime<Utc>,
}

/// Log position range for filtering exports.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionRange {
    /// Start position (inclusive).
    pub start: Offset,
    /// End position (inclusive).
    pub end: Offset,
}

/// Rule for transforming data during export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransformationRule {
    /// Field to transform (supports wildcards like "*.ssn").
    pub field_pattern: String,
    /// Type of transformation to apply.
    pub transformation: TransformationType,
}

impl TransformationRule {
    /// Creates a new transformation rule.
    pub fn new(field_pattern: impl Into<String>, transformation: TransformationType) -> Self {
        Self {
            field_pattern: field_pattern.into(),
            transformation,
        }
    }

    /// Checks if this rule matches a field.
    pub fn matches_field(&self, field: &str) -> bool {
        if self.field_pattern == "*" {
            return true;
        }
        if self.field_pattern.starts_with("*.") {
            let suffix = &self.field_pattern[1..];
            return field.ends_with(suffix);
        }
        if self.field_pattern.ends_with(".*") {
            let prefix = &self.field_pattern[..self.field_pattern.len() - 2];
            return field.starts_with(prefix);
        }
        field == self.field_pattern
    }
}

/// Types of data transformations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TransformationType {
    /// Completely remove the field.
    Redact,
    /// Replace with asterisks, keeping length.
    Mask,
    /// Partially mask (e.g., show last 4 digits).
    PartialMask {
        /// Characters to show at the start.
        show_start: usize,
        /// Characters to show at the end.
        show_end: usize,
    },
    /// Replace with pseudonymous identifier.
    Pseudonymize,
    /// Apply k-anonymity generalization.
    Generalize {
        /// Bucket size for numeric values.
        bucket_size: Option<u64>,
    },
    /// Hash the value.
    Hash,
    /// Encrypt the value (requires key).
    Encrypt,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_full_scope() {
        let scope = ExportScope::full();
        assert!(scope.allows_table("users"));
        assert!(scope.allows_field("name"));
        assert!(scope.allows_stream(StreamId::new(1)));
    }

    #[test]
    fn test_limited_tables() {
        let scope = ExportScope::tables(["users", "orders"]);
        assert!(scope.allows_table("users"));
        assert!(scope.allows_table("orders"));
        assert!(!scope.allows_table("secrets"));
    }

    #[test]
    fn test_field_exclusion() {
        let scope = ExportScope::full().exclude_fields(["ssn", "password"]);
        assert!(scope.allows_field("name"));
        assert!(scope.allows_field("email"));
        assert!(!scope.allows_field("ssn"));
        assert!(!scope.allows_field("password"));
    }

    #[test]
    fn test_transformation_matching() {
        let rule = TransformationRule::new("*.ssn", TransformationType::Redact);
        assert!(rule.matches_field("user.ssn"));
        assert!(rule.matches_field("patient.ssn"));
        assert!(!rule.matches_field("ssn_backup"));

        let rule2 = TransformationRule::new("user.*", TransformationType::Mask);
        assert!(rule2.matches_field("user.name"));
        assert!(rule2.matches_field("user.email"));
        assert!(!rule2.matches_field("admin.name"));

        let rule3 = TransformationRule::new("password", TransformationType::Redact);
        assert!(rule3.matches_field("password"));
        assert!(!rule3.matches_field("user.password"));
    }

    #[test]
    fn test_time_range() {
        use chrono::Duration;

        let now = Utc::now();
        let scope =
            ExportScope::full().with_time_range(now - Duration::days(7), now + Duration::days(1));

        assert!(scope.allows_time(now));
        assert!(scope.allows_time(now - Duration::days(3)));
        assert!(!scope.allows_time(now - Duration::days(30)));
    }
}
