//! Consent ledger for tracking data sharing agreements.
//!
//! Records what data was shared, with whom, when, and for what purpose.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use craton_types::TenantId;

use crate::scope::ExportScope;
use crate::token::TokenId;

/// Unique identifier for a consent record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConsentId(u64);

impl ConsentId {
    /// Creates a new consent ID.
    pub fn new(id: u64) -> Self {
        Self(id)
    }
}

impl std::fmt::Display for ConsentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "consent-{}", self.0)
    }
}

/// Status of a consent agreement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConsentStatus {
    /// Consent is active.
    Active,
    /// Consent has been withdrawn by the data subject.
    Withdrawn,
    /// Consent has expired.
    Expired,
    /// Consent was revoked by an administrator.
    Revoked,
}

/// A consent record documenting agreement to share data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsentRecord {
    /// Unique consent identifier.
    pub id: ConsentId,
    /// Tenant this consent belongs to.
    pub tenant_id: TenantId,
    /// Data subject identifier (who the data is about).
    pub data_subject: String,
    /// Data recipient (who the data is being shared with).
    pub recipient: String,
    /// Purpose of the data sharing.
    pub purpose: String,
    /// Scope of data being shared.
    pub scope: ExportScope,
    /// When consent was given.
    pub consented_at: DateTime<Utc>,
    /// When consent expires (if applicable).
    pub expires_at: Option<DateTime<Utc>>,
    /// Current status.
    pub status: ConsentStatus,
    /// Status change history.
    pub history: Vec<ConsentHistoryEntry>,
    /// Legal basis for processing (GDPR compliance).
    pub legal_basis: Option<LegalBasis>,
    /// Any additional metadata.
    pub metadata: HashMap<String, String>,
}

impl ConsentRecord {
    /// Creates a new active consent record.
    pub fn new(
        id: ConsentId,
        tenant_id: TenantId,
        data_subject: impl Into<String>,
        recipient: impl Into<String>,
        purpose: impl Into<String>,
        scope: ExportScope,
    ) -> Self {
        Self {
            id,
            tenant_id,
            data_subject: data_subject.into(),
            recipient: recipient.into(),
            purpose: purpose.into(),
            scope,
            consented_at: Utc::now(),
            expires_at: None,
            status: ConsentStatus::Active,
            history: vec![ConsentHistoryEntry {
                timestamp: Utc::now(),
                action: ConsentAction::Granted,
                actor: None,
                reason: None,
            }],
            legal_basis: None,
            metadata: HashMap::new(),
        }
    }

    /// Sets the expiration time.
    pub fn with_expiration(mut self, expires_at: DateTime<Utc>) -> Self {
        self.expires_at = Some(expires_at);
        self
    }

    /// Sets the legal basis.
    pub fn with_legal_basis(mut self, basis: LegalBasis) -> Self {
        self.legal_basis = Some(basis);
        self
    }

    /// Adds metadata.
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Checks if consent is currently valid.
    pub fn is_valid(&self) -> bool {
        if self.status != ConsentStatus::Active {
            return false;
        }
        if let Some(expires) = self.expires_at {
            if Utc::now() >= expires {
                return false;
            }
        }
        true
    }

    /// Withdraws consent.
    pub fn withdraw(&mut self, reason: Option<String>) {
        self.status = ConsentStatus::Withdrawn;
        self.history.push(ConsentHistoryEntry {
            timestamp: Utc::now(),
            action: ConsentAction::Withdrawn,
            actor: None,
            reason,
        });
    }

    /// Revokes consent (administrative action).
    pub fn revoke(&mut self, actor: impl Into<String>, reason: impl Into<String>) {
        self.status = ConsentStatus::Revoked;
        self.history.push(ConsentHistoryEntry {
            timestamp: Utc::now(),
            action: ConsentAction::Revoked,
            actor: Some(actor.into()),
            reason: Some(reason.into()),
        });
    }

    /// Marks consent as expired.
    pub fn expire(&mut self) {
        self.status = ConsentStatus::Expired;
        self.history.push(ConsentHistoryEntry {
            timestamp: Utc::now(),
            action: ConsentAction::Expired,
            actor: None,
            reason: None,
        });
    }
}

/// Entry in the consent history log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsentHistoryEntry {
    /// When the action occurred.
    pub timestamp: DateTime<Utc>,
    /// What action was taken.
    pub action: ConsentAction,
    /// Who took the action (if applicable).
    pub actor: Option<String>,
    /// Reason for the action (if applicable).
    pub reason: Option<String>,
}

/// Actions that can be taken on consent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConsentAction {
    /// Consent was granted.
    Granted,
    /// Consent was withdrawn by data subject.
    Withdrawn,
    /// Consent was revoked by administrator.
    Revoked,
    /// Consent expired.
    Expired,
    /// Consent scope was modified.
    Modified,
}

/// Legal basis for processing under GDPR.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LegalBasis {
    /// Data subject has given consent (Art. 6(1)(a)).
    Consent,
    /// Processing necessary for contract performance (Art. 6(1)(b)).
    Contract,
    /// Processing necessary for legal obligation (Art. 6(1)(c)).
    LegalObligation,
    /// Processing necessary for vital interests (Art. 6(1)(d)).
    VitalInterests,
    /// Processing necessary for public interest (Art. 6(1)(e)).
    PublicInterest,
    /// Processing necessary for legitimate interests (Art. 6(1)(f)).
    LegitimateInterests { description: String },
}

/// Record of a data export operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportRecord {
    /// Unique export identifier.
    pub id: u64,
    /// Associated consent (if any).
    pub consent_id: Option<ConsentId>,
    /// Token used for the export.
    pub token_id: TokenId,
    /// Tenant ID.
    pub tenant_id: TenantId,
    /// When the export occurred.
    pub exported_at: DateTime<Utc>,
    /// Recipient of the export.
    pub recipient: String,
    /// Query or operation that was performed.
    pub operation: String,
    /// Number of rows exported.
    pub row_count: u64,
    /// Hash of the exported data (for verification).
    pub data_hash: [u8; 32],
    /// Transformations that were applied.
    pub transformations_applied: Vec<String>,
}

/// Consent ledger for managing consent records.
#[derive(Debug, Default)]
pub struct ConsentLedger {
    consents: HashMap<ConsentId, ConsentRecord>,
    exports: Vec<ExportRecord>,
    next_consent_id: u64,
    next_export_id: u64,
}

impl ConsentLedger {
    /// Creates a new empty consent ledger.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a new consent.
    pub fn record_consent(&mut self, mut consent: ConsentRecord) -> ConsentId {
        let id = ConsentId::new(self.next_consent_id);
        self.next_consent_id += 1;
        consent.id = id;
        self.consents.insert(id, consent);
        id
    }

    /// Gets a consent record.
    pub fn get_consent(&self, id: ConsentId) -> Option<&ConsentRecord> {
        self.consents.get(&id)
    }

    /// Gets a mutable consent record.
    pub fn get_consent_mut(&mut self, id: ConsentId) -> Option<&mut ConsentRecord> {
        self.consents.get_mut(&id)
    }

    /// Finds valid consents for a data subject and recipient.
    pub fn find_valid_consent(
        &self,
        tenant_id: TenantId,
        data_subject: &str,
        recipient: &str,
    ) -> Option<&ConsentRecord> {
        self.consents.values().find(|c| {
            c.tenant_id == tenant_id
                && c.data_subject == data_subject
                && c.recipient == recipient
                && c.is_valid()
        })
    }

    /// Records a data export.
    pub fn record_export(&mut self, mut export: ExportRecord) -> u64 {
        let id = self.next_export_id;
        self.next_export_id += 1;
        export.id = id;
        self.exports.push(export);
        id
    }

    /// Gets all exports for a consent.
    pub fn get_exports_for_consent(&self, consent_id: ConsentId) -> Vec<&ExportRecord> {
        self.exports
            .iter()
            .filter(|e| e.consent_id == Some(consent_id))
            .collect()
    }

    /// Gets all exports for a token.
    pub fn get_exports_for_token(&self, token_id: TokenId) -> Vec<&ExportRecord> {
        self.exports
            .iter()
            .filter(|e| e.token_id == token_id)
            .collect()
    }

    /// Lists all consents for a tenant.
    pub fn list_consents(&self, tenant_id: TenantId) -> Vec<&ConsentRecord> {
        self.consents
            .values()
            .filter(|c| c.tenant_id == tenant_id)
            .collect()
    }

    /// Lists all exports for a tenant.
    pub fn list_exports(&self, tenant_id: TenantId) -> Vec<&ExportRecord> {
        self.exports
            .iter()
            .filter(|e| e.tenant_id == tenant_id)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_consent_creation() {
        let consent = ConsentRecord::new(
            ConsentId::new(1),
            TenantId::new(1),
            "patient-123",
            "research-org",
            "Clinical research study",
            ExportScope::full(),
        );

        assert!(consent.is_valid());
        assert_eq!(consent.status, ConsentStatus::Active);
        assert_eq!(consent.history.len(), 1);
    }

    #[test]
    fn test_consent_withdrawal() {
        let mut consent = ConsentRecord::new(
            ConsentId::new(1),
            TenantId::new(1),
            "patient-123",
            "research-org",
            "Clinical research",
            ExportScope::full(),
        );

        assert!(consent.is_valid());

        consent.withdraw(Some("Changed my mind".to_string()));

        assert!(!consent.is_valid());
        assert_eq!(consent.status, ConsentStatus::Withdrawn);
        assert_eq!(consent.history.len(), 2);
    }

    #[test]
    fn test_consent_ledger() {
        let mut ledger = ConsentLedger::new();
        let tenant = TenantId::new(1);

        let consent = ConsentRecord::new(
            ConsentId::new(0), // Will be replaced
            tenant,
            "patient-123",
            "research-org",
            "Research",
            ExportScope::full(),
        );

        let id = ledger.record_consent(consent);

        let found = ledger.find_valid_consent(tenant, "patient-123", "research-org");
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, id);

        // Withdraw and verify no longer found
        ledger.get_consent_mut(id).unwrap().withdraw(None);

        let found = ledger.find_valid_consent(tenant, "patient-123", "research-org");
        assert!(found.is_none());
    }
}
