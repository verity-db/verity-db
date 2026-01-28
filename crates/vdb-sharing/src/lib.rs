//! # vdb-sharing: Secure data sharing and export for `VerityDB`
//!
//! This crate provides infrastructure for secure, compliant data sharing:
//!
//! - **Token Management**: Time-bound, scope-limited access tokens
//! - **Export Scopes**: Define what data can be exported and how
//! - **Transformations**: Anonymize, pseudonymize, mask, and redact data
//! - **Consent Ledger**: Track consent agreements and export history
//!
//! ## Usage
//!
//! ```ignore
//! use vdb_sharing::{
//!     TokenStore, AccessToken, ExportScope, TransformationRule,
//!     TransformationType, ConsentLedger, ConsentRecord,
//! };
//! use vdb_types::TenantId;
//! use chrono::Duration;
//!
//! // Create a token store
//! let mut tokens = TokenStore::new();
//!
//! // Define export scope with transformations
//! let scope = ExportScope::tables(["patients"])
//!     .exclude_fields(["ssn", "dob"])
//!     .with_transformation(TransformationRule::new(
//!         "name",
//!         TransformationType::Pseudonymize,
//!     ))
//!     .with_max_rows(1000);
//!
//! // Issue a time-limited token
//! let token = AccessToken::new(
//!     TenantId::new(1),
//!     "Research export for Study XYZ",
//!     scope,
//!     Duration::days(7),
//! );
//! let token_id = tokens.issue(token);
//!
//! // Use the token for export
//! let scope = tokens.use_token(token_id)?;
//! ```
//!
//! ## Consent Management
//!
//! ```ignore
//! use vdb_sharing::{ConsentLedger, ConsentRecord, ExportScope, LegalBasis};
//! use vdb_types::TenantId;
//!
//! let mut ledger = ConsentLedger::new();
//!
//! // Record consent
//! let consent = ConsentRecord::new(
//!     ConsentId::new(0),
//!     TenantId::new(1),
//!     "patient-123",
//!     "research-org",
//!     "Clinical research study ABC",
//!     ExportScope::tables(["encounters", "diagnoses"]),
//! ).with_legal_basis(LegalBasis::Consent);
//!
//! ledger.record_consent(consent);
//! ```

mod consent;
mod error;
mod scope;
mod token;
mod transform;

#[cfg(test)]
mod tests;

// Re-export public types
pub use consent::{
    ConsentAction, ConsentHistoryEntry, ConsentId, ConsentLedger, ConsentRecord, ConsentStatus,
    ExportRecord, LegalBasis,
};
pub use error::{SharingError, SharingResult};
pub use scope::{ExportScope, PositionRange, TimeRange, TransformationRule, TransformationType};
pub use token::{AccessToken, TokenId, TokenStore};
pub use transform::{Transformer, apply_transformation, transform_record};
