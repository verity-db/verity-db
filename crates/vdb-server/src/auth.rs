//! Authentication module for JWT and API key validation.
//!
//! Supports multiple authentication methods:
//! - JWT tokens (for user sessions)
//! - API keys (for service accounts)

use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use vdb_types::TenantId;

use crate::error::{ServerError, ServerResult};

/// Authentication mode for the server.
#[derive(Debug, Clone, Default)]
pub enum AuthMode {
    /// No authentication required.
    #[default]
    None,
    /// JWT token authentication.
    Jwt(JwtConfig),
    /// API key authentication.
    ApiKey(ApiKeyConfig),
    /// Both JWT and API key authentication (either is accepted).
    Both {
        jwt: JwtConfig,
        api_key: ApiKeyConfig,
    },
}

/// JWT configuration.
#[derive(Debug, Clone)]
pub struct JwtConfig {
    /// Secret key for signing/verifying tokens.
    secret: String,
    /// Token expiration duration.
    pub expiration: Duration,
    /// Issuer claim.
    pub issuer: String,
    /// Audience claims.
    pub audience: Vec<String>,
}

impl JwtConfig {
    /// Creates a new JWT configuration.
    pub fn new(secret: impl Into<String>) -> Self {
        Self {
            secret: secret.into(),
            expiration: Duration::from_secs(3600), // 1 hour
            issuer: "veritydb".to_string(),
            audience: vec!["veritydb".to_string()],
        }
    }

    /// Sets the token expiration duration.
    #[must_use]
    pub fn with_expiration(mut self, expiration: Duration) -> Self {
        self.expiration = expiration;
        self
    }

    /// Sets the issuer claim.
    #[must_use]
    pub fn with_issuer(mut self, issuer: impl Into<String>) -> Self {
        self.issuer = issuer.into();
        self
    }

    /// Adds an audience claim.
    #[must_use]
    pub fn with_audience(mut self, audience: impl Into<String>) -> Self {
        self.audience.push(audience.into());
        self
    }
}

/// API key configuration.
#[derive(Debug, Clone, Default)]
pub struct ApiKeyConfig {
    /// Whether to enable API key authentication.
    pub enabled: bool,
}

impl ApiKeyConfig {
    /// Creates a new API key configuration.
    pub fn new() -> Self {
        Self { enabled: true }
    }
}

/// JWT claims structure.
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    /// Subject (user ID).
    pub sub: String,
    /// Tenant ID the user belongs to.
    pub tenant_id: u64,
    /// User roles.
    pub roles: Vec<String>,
    /// Issued at timestamp (seconds since epoch).
    pub iat: u64,
    /// Expiration timestamp (seconds since epoch).
    pub exp: u64,
    /// Issuer.
    pub iss: String,
    /// Audience.
    pub aud: Vec<String>,
}

/// Authenticated identity after successful authentication.
#[derive(Debug, Clone)]
pub struct AuthenticatedIdentity {
    /// User or service account ID.
    pub subject: String,
    /// Tenant ID.
    pub tenant_id: TenantId,
    /// Roles/permissions.
    pub roles: Vec<String>,
    /// How the identity was authenticated.
    pub method: AuthMethod,
}

/// Authentication method used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMethod {
    /// JWT token.
    Jwt,
    /// API key.
    ApiKey,
    /// No authentication (anonymous).
    Anonymous,
}

/// Authentication service that handles token validation and API key verification.
pub struct AuthService {
    /// Authentication mode.
    mode: AuthMode,
    /// API key store (in-memory for now, could be backed by database).
    api_keys: RwLock<HashMap<String, ApiKeyEntry>>,
}

/// An API key entry.
#[derive(Debug, Clone)]
struct ApiKeyEntry {
    /// The hashed API key.
    #[allow(dead_code)]
    key_hash: String,
    /// Subject (service account ID).
    subject: String,
    /// Tenant ID.
    tenant_id: TenantId,
    /// Roles.
    roles: Vec<String>,
    /// Expiration (optional).
    expires_at: Option<SystemTime>,
}

impl AuthService {
    /// Creates a new authentication service.
    pub fn new(mode: AuthMode) -> Self {
        Self {
            mode,
            api_keys: RwLock::new(HashMap::new()),
        }
    }

    /// Authenticates a request using the provided token.
    ///
    /// The token can be either a JWT or an API key, depending on the configured mode.
    pub fn authenticate(&self, token: Option<&str>) -> ServerResult<AuthenticatedIdentity> {
        match &self.mode {
            AuthMode::None => Ok(AuthenticatedIdentity {
                subject: "anonymous".to_string(),
                tenant_id: TenantId::new(0),
                roles: vec![],
                method: AuthMethod::Anonymous,
            }),

            AuthMode::Jwt(config) => {
                let token = token.ok_or(ServerError::Unauthorized("missing token".to_string()))?;
                self.validate_jwt(token, config)
            }

            AuthMode::ApiKey(_) => {
                let token =
                    token.ok_or(ServerError::Unauthorized("missing API key".to_string()))?;
                self.validate_api_key(token)
            }

            AuthMode::Both { jwt, api_key: _ } => {
                let token =
                    token.ok_or(ServerError::Unauthorized("missing credentials".to_string()))?;

                // Try JWT first, then API key
                if token.contains('.') {
                    // Looks like a JWT (has dots)
                    self.validate_jwt(token, jwt)
                } else {
                    // Try as API key
                    self.validate_api_key(token)
                }
            }
        }
    }

    /// Validates a JWT token.
    #[allow(clippy::unused_self)] // May need self in the future for token blacklisting
    fn validate_jwt(&self, token: &str, config: &JwtConfig) -> ServerResult<AuthenticatedIdentity> {
        let mut validation = Validation::default();
        validation.set_issuer(&[&config.issuer]);
        validation.set_audience(&config.audience);

        let token_data = decode::<Claims>(
            token,
            &DecodingKey::from_secret(config.secret.as_bytes()),
            &validation,
        )
        .map_err(|e| ServerError::Unauthorized(format!("invalid JWT: {e}")))?;

        let claims = token_data.claims;

        Ok(AuthenticatedIdentity {
            subject: claims.sub,
            tenant_id: TenantId::new(claims.tenant_id),
            roles: claims.roles,
            method: AuthMethod::Jwt,
        })
    }

    /// Validates an API key.
    fn validate_api_key(&self, key: &str) -> ServerResult<AuthenticatedIdentity> {
        let keys = self
            .api_keys
            .read()
            .map_err(|_| ServerError::Unauthorized("lock poisoned".to_string()))?;

        let entry = keys
            .get(key)
            .ok_or_else(|| ServerError::Unauthorized("invalid API key".to_string()))?;

        // Check expiration
        if let Some(expires_at) = entry.expires_at {
            if SystemTime::now() > expires_at {
                return Err(ServerError::Unauthorized("API key expired".to_string()));
            }
        }

        Ok(AuthenticatedIdentity {
            subject: entry.subject.clone(),
            tenant_id: entry.tenant_id,
            roles: entry.roles.clone(),
            method: AuthMethod::ApiKey,
        })
    }

    /// Registers an API key (for testing or initial setup).
    pub fn register_api_key(
        &self,
        key: impl Into<String>,
        subject: impl Into<String>,
        tenant_id: TenantId,
        roles: Vec<String>,
        expires_at: Option<SystemTime>,
    ) -> ServerResult<()> {
        let key = key.into();
        // In production, we would hash the key before storing
        let key_hash = key.clone(); // Simplified for now

        let entry = ApiKeyEntry {
            key_hash,
            subject: subject.into(),
            tenant_id,
            roles,
            expires_at,
        };

        self.api_keys
            .write()
            .map_err(|_| ServerError::Unauthorized("lock poisoned".to_string()))?
            .insert(key, entry);

        Ok(())
    }

    /// Creates a JWT token for a user.
    pub fn create_jwt(
        &self,
        subject: &str,
        tenant_id: TenantId,
        roles: Vec<String>,
    ) -> ServerResult<String> {
        let config = match &self.mode {
            AuthMode::Jwt(c) => c,
            AuthMode::Both { jwt, .. } => jwt,
            _ => {
                return Err(ServerError::Unauthorized(
                    "JWT authentication not configured".to_string(),
                ));
            }
        };

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| ServerError::Unauthorized(format!("time error: {e}")))?;

        let claims = Claims {
            sub: subject.to_string(),
            tenant_id: u64::from(tenant_id),
            roles,
            iat: now.as_secs(),
            exp: (now + config.expiration).as_secs(),
            iss: config.issuer.clone(),
            aud: config.audience.clone(),
        };

        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(config.secret.as_bytes()),
        )
        .map_err(|e| ServerError::Unauthorized(format!("failed to create JWT: {e}")))
    }

    /// Revokes an API key.
    pub fn revoke_api_key(&self, key: &str) -> ServerResult<bool> {
        let mut keys = self
            .api_keys
            .write()
            .map_err(|_| ServerError::Unauthorized("lock poisoned".to_string()))?;

        Ok(keys.remove(key).is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_mode_none() {
        let service = AuthService::new(AuthMode::None);
        let identity = service.authenticate(None).unwrap();
        assert_eq!(identity.method, AuthMethod::Anonymous);
        assert_eq!(identity.subject, "anonymous");
    }

    #[test]
    fn test_jwt_creation_and_validation() {
        let config = JwtConfig::new("test-secret-key-that-is-long-enough");
        let service = AuthService::new(AuthMode::Jwt(config));

        // Create a token
        let token = service
            .create_jwt("user123", TenantId::new(1), vec!["admin".to_string()])
            .unwrap();

        // Validate the token
        let identity = service.authenticate(Some(&token)).unwrap();
        assert_eq!(identity.subject, "user123");
        assert_eq!(identity.tenant_id, TenantId::new(1));
        assert_eq!(identity.roles, vec!["admin"]);
        assert_eq!(identity.method, AuthMethod::Jwt);
    }

    #[test]
    fn test_jwt_missing_token() {
        let config = JwtConfig::new("test-secret");
        let service = AuthService::new(AuthMode::Jwt(config));

        let result = service.authenticate(None);
        assert!(result.is_err());
    }

    #[test]
    fn test_api_key_authentication() {
        let service = AuthService::new(AuthMode::ApiKey(ApiKeyConfig::new()));

        // Register an API key
        service
            .register_api_key(
                "test-api-key",
                "service-account-1",
                TenantId::new(2),
                vec!["read".to_string()],
                None,
            )
            .unwrap();

        // Validate the API key
        let identity = service.authenticate(Some("test-api-key")).unwrap();
        assert_eq!(identity.subject, "service-account-1");
        assert_eq!(identity.tenant_id, TenantId::new(2));
        assert_eq!(identity.method, AuthMethod::ApiKey);
    }

    #[test]
    fn test_api_key_invalid() {
        let service = AuthService::new(AuthMode::ApiKey(ApiKeyConfig::new()));

        let result = service.authenticate(Some("invalid-key"));
        assert!(result.is_err());
    }

    #[test]
    fn test_api_key_revocation() {
        let service = AuthService::new(AuthMode::ApiKey(ApiKeyConfig::new()));

        service
            .register_api_key("key-to-revoke", "test", TenantId::new(1), vec![], None)
            .unwrap();

        // Key should work
        assert!(service.authenticate(Some("key-to-revoke")).is_ok());

        // Revoke it
        assert!(service.revoke_api_key("key-to-revoke").unwrap());

        // Key should no longer work
        assert!(service.authenticate(Some("key-to-revoke")).is_err());
    }
}
