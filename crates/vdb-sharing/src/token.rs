//! Access token management for data sharing.
//!
//! Tokens provide time-bound, scope-limited access to exported data.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Duration, Utc};
use rand::Rng;
use serde::{Deserialize, Serialize};
use vdb_types::TenantId;

use crate::error::{SharingError, SharingResult};
use crate::scope::ExportScope;

/// Unique identifier for an access token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TokenId([u8; 16]);

impl TokenId {
    /// Generates a new random token ID.
    pub fn generate() -> Self {
        let mut bytes = [0u8; 16];
        rand::thread_rng().fill(&mut bytes);
        Self(bytes)
    }

    /// Creates a token ID from bytes.
    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Returns the token ID as bytes.
    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    /// Returns the token ID as a hex string.
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
}

impl std::fmt::Display for TokenId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

/// Access token for data export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessToken {
    /// Unique token identifier.
    pub id: TokenId,
    /// Tenant this token belongs to.
    pub tenant_id: TenantId,
    /// Description of what this token is for.
    pub description: String,
    /// Scope of data access allowed by this token.
    pub scope: ExportScope,
    /// When the token was created.
    pub created_at: DateTime<Utc>,
    /// When the token expires.
    pub expires_at: DateTime<Utc>,
    /// Whether this is a one-time use token.
    pub one_time: bool,
    /// Number of times the token has been used.
    pub use_count: u64,
    /// Maximum number of uses (0 = unlimited, unless `one_time`).
    pub max_uses: u64,
    /// Whether the token has been revoked.
    pub revoked: bool,
    /// When the token was revoked (if applicable).
    pub revoked_at: Option<DateTime<Utc>>,
    /// Who revoked the token (if applicable).
    pub revoked_by: Option<String>,
}

impl AccessToken {
    /// Creates a new access token.
    pub fn new(
        tenant_id: TenantId,
        description: impl Into<String>,
        scope: ExportScope,
        ttl: Duration,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: TokenId::generate(),
            tenant_id,
            description: description.into(),
            scope,
            created_at: now,
            expires_at: now + ttl,
            one_time: false,
            use_count: 0,
            max_uses: 0,
            revoked: false,
            revoked_at: None,
            revoked_by: None,
        }
    }

    /// Creates a one-time use token.
    pub fn one_time(
        tenant_id: TenantId,
        description: impl Into<String>,
        scope: ExportScope,
        ttl: Duration,
    ) -> Self {
        let mut token = Self::new(tenant_id, description, scope, ttl);
        token.one_time = true;
        token.max_uses = 1;
        token
    }

    /// Sets the maximum number of uses for this token.
    pub fn with_max_uses(mut self, max: u64) -> Self {
        self.max_uses = max;
        self
    }

    /// Checks if the token is valid for use.
    pub fn is_valid(&self) -> bool {
        !self.revoked && !self.is_expired() && !self.is_exhausted()
    }

    /// Checks if the token has expired.
    pub fn is_expired(&self) -> bool {
        Utc::now() >= self.expires_at
    }

    /// Checks if the token has been used up.
    pub fn is_exhausted(&self) -> bool {
        if self.one_time && self.use_count > 0 {
            return true;
        }
        if self.max_uses > 0 && self.use_count >= self.max_uses {
            return true;
        }
        false
    }

    /// Records a use of this token.
    pub fn record_use(&mut self) {
        self.use_count += 1;
    }

    /// Revokes this token.
    pub fn revoke(&mut self, by: impl Into<String>) {
        self.revoked = true;
        self.revoked_at = Some(Utc::now());
        self.revoked_by = Some(by.into());
    }

    /// Validates that this token can be used and returns an error if not.
    pub fn validate(&self) -> SharingResult<()> {
        if self.revoked {
            return Err(SharingError::TokenRevoked);
        }
        if self.is_expired() {
            return Err(SharingError::TokenExpired);
        }
        if self.is_exhausted() {
            return Err(SharingError::TokenExpired);
        }
        Ok(())
    }
}

/// Token store for managing access tokens.
#[derive(Debug, Default)]
pub struct TokenStore {
    tokens: HashMap<TokenId, AccessToken>,
    by_tenant: HashMap<TenantId, HashSet<TokenId>>,
}

impl TokenStore {
    /// Creates a new empty token store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Issues a new access token.
    pub fn issue(&mut self, token: AccessToken) -> TokenId {
        let id = token.id;
        let tenant_id = token.tenant_id;

        self.tokens.insert(id, token);
        self.by_tenant.entry(tenant_id).or_default().insert(id);

        id
    }

    /// Gets a token by ID.
    pub fn get(&self, id: TokenId) -> Option<&AccessToken> {
        self.tokens.get(&id)
    }

    /// Gets a mutable reference to a token by ID.
    pub fn get_mut(&mut self, id: TokenId) -> Option<&mut AccessToken> {
        self.tokens.get_mut(&id)
    }

    /// Validates and uses a token.
    ///
    /// Returns the token scope if valid, or an error if not.
    pub fn use_token(&mut self, id: TokenId) -> SharingResult<ExportScope> {
        let token = self
            .tokens
            .get_mut(&id)
            .ok_or(SharingError::TokenNotFound)?;

        token.validate()?;
        token.record_use();

        Ok(token.scope.clone())
    }

    /// Revokes a token.
    pub fn revoke(&mut self, id: TokenId, by: impl Into<String>) -> SharingResult<()> {
        let token = self
            .tokens
            .get_mut(&id)
            .ok_or(SharingError::TokenNotFound)?;

        token.revoke(by);
        Ok(())
    }

    /// Lists all tokens for a tenant.
    pub fn list_for_tenant(&self, tenant_id: TenantId) -> Vec<&AccessToken> {
        self.by_tenant
            .get(&tenant_id)
            .map(|ids| ids.iter().filter_map(|id| self.tokens.get(id)).collect())
            .unwrap_or_default()
    }

    /// Lists all active (non-revoked, non-expired) tokens for a tenant.
    pub fn list_active_for_tenant(&self, tenant_id: TenantId) -> Vec<&AccessToken> {
        self.list_for_tenant(tenant_id)
            .into_iter()
            .filter(|t| t.is_valid())
            .collect()
    }

    /// Removes expired and revoked tokens.
    pub fn cleanup(&mut self) {
        let to_remove: Vec<TokenId> = self
            .tokens
            .iter()
            .filter(|(_, t)| t.revoked || t.is_expired())
            .map(|(id, _)| *id)
            .collect();

        for id in to_remove {
            if let Some(token) = self.tokens.remove(&id) {
                if let Some(tenant_tokens) = self.by_tenant.get_mut(&token.tenant_id) {
                    tenant_tokens.remove(&id);
                }
            }
        }
    }
}

// Include hex encoding helper
mod hex {
    const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";

    pub fn encode(bytes: [u8; 16]) -> String {
        let mut result = String::with_capacity(32);
        for byte in bytes {
            result.push(HEX_CHARS[(byte >> 4) as usize] as char);
            result.push(HEX_CHARS[(byte & 0xf) as usize] as char);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_id_generation() {
        let id1 = TokenId::generate();
        let id2 = TokenId::generate();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_token_id_hex() {
        let id = TokenId::from_bytes([
            0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
            0xcd, 0xef,
        ]);
        assert_eq!(id.to_hex(), "0123456789abcdef0123456789abcdef");
    }

    #[test]
    fn test_token_creation() {
        let tenant = TenantId::new(1);
        let scope = ExportScope::full();
        let token = AccessToken::new(tenant, "test token", scope, Duration::hours(24));

        assert!(!token.is_expired());
        assert!(!token.revoked);
        assert!(token.is_valid());
    }

    #[test]
    fn test_one_time_token() {
        let tenant = TenantId::new(1);
        let scope = ExportScope::full();
        let mut token = AccessToken::one_time(tenant, "one-time token", scope, Duration::hours(1));

        assert!(token.is_valid());

        token.record_use();
        assert!(!token.is_valid());
        assert!(token.is_exhausted());
    }

    #[test]
    fn test_token_revocation() {
        let tenant = TenantId::new(1);
        let scope = ExportScope::full();
        let mut token = AccessToken::new(tenant, "test", scope, Duration::hours(24));

        assert!(token.is_valid());

        token.revoke("admin");
        assert!(!token.is_valid());
        assert!(token.revoked);
        assert!(token.revoked_at.is_some());
        assert_eq!(token.revoked_by.as_deref(), Some("admin"));
    }

    #[test]
    fn test_token_store() {
        let mut store = TokenStore::new();
        let tenant = TenantId::new(1);
        let scope = ExportScope::full();

        let token = AccessToken::new(tenant, "test", scope.clone(), Duration::hours(24));
        let id = store.issue(token);

        // Can use the token
        let result = store.use_token(id);
        assert!(result.is_ok());

        // Can revoke
        store.revoke(id, "admin").unwrap();

        // Cannot use revoked token
        let result = store.use_token(id);
        assert!(matches!(result, Err(SharingError::TokenRevoked)));
    }
}
