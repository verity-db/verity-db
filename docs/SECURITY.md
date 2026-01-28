# Security Guide

This document covers security configuration for Craton and the cloud platform, including authentication, authorization, TLS, and tenant isolation.

## Table of Contents

1. [Security Model](#security-model)
2. [TLS Configuration](#tls-configuration)
3. [Authentication](#authentication)
4. [Authorization (RBAC)](#authorization-rbac)
5. [Tenant Isolation](#tenant-isolation)
6. [Audit Logging](#audit-logging)
7. [Security Hardening](#security-hardening)
8. [Incident Response](#incident-response)

---

## Security Model

Craton's security is built on defense in depth with multiple layers:

```
┌─────────────────────────────────────────────────────────────────┐
│                      Security Layers                             │
│  ┌─────────────────────────────────────────────────────────────┐│
│  │ Layer 1: Network Security                                    ││
│  │ - TLS 1.3 for all connections                               ││
│  │ - mTLS for service-to-service communication                 ││
│  │ - Network policies (Kubernetes/firewall)                     ││
│  └─────────────────────────────────────────────────────────────┘│
│  ┌─────────────────────────────────────────────────────────────┐│
│  │ Layer 2: Authentication                                      ││
│  │ - JWT tokens for API access                                  ││
│  │ - API keys for service accounts                              ││
│  │ - WebAuthn/Passkeys for users                                ││
│  │ - OAuth for identity providers                               ││
│  └─────────────────────────────────────────────────────────────┘│
│  ┌─────────────────────────────────────────────────────────────┐│
│  │ Layer 3: Authorization                                       ││
│  │ - RBAC at organization level                                 ││
│  │ - Tenant isolation at data level                             ││
│  │ - Resource-level permissions                                 ││
│  └─────────────────────────────────────────────────────────────┘│
│  ┌─────────────────────────────────────────────────────────────┐│
│  │ Layer 4: Data Protection                                     ││
│  │ - Encryption at rest (AES-256-GCM)                          ││
│  │ - Hash chains for integrity                                  ││
│  │ - Field-level encryption for PII                             ││
│  └─────────────────────────────────────────────────────────────┘│
│  ┌─────────────────────────────────────────────────────────────┐│
│  │ Layer 5: Audit & Compliance                                  ││
│  │ - Immutable audit log                                        ││
│  │ - Cryptographic proofs                                       ││
│  │ - Access logging                                             ││
│  └─────────────────────────────────────────────────────────────┘│
└─────────────────────────────────────────────────────────────────┘
```

---

## TLS Configuration

### Minimum Requirements

- TLS 1.3 required (TLS 1.2 disabled by default)
- Strong cipher suites only
- Certificate chain validation enabled
- OCSP stapling recommended

### Server Configuration

```rust
// TLS configuration in craton-server
pub struct TlsConfig {
    /// Path to certificate file (PEM format)
    pub cert_path: PathBuf,
    /// Path to private key file (PEM format)
    pub key_path: PathBuf,
    /// Path to CA certificate for client verification (mTLS)
    pub ca_cert_path: Option<PathBuf>,
    /// Require client certificate (mTLS mode)
    pub require_client_cert: bool,
    /// Minimum TLS version (default: TLS 1.3)
    pub min_version: TlsVersion,
}

impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            cert_path: PathBuf::from("/etc/craton/certs/server.crt"),
            key_path: PathBuf::from("/etc/craton/certs/server.key"),
            ca_cert_path: None,
            require_client_cert: false,
            min_version: TlsVersion::TLS13,
        }
    }
}
```

### Cipher Suites

Allowed cipher suites (TLS 1.3):
- `TLS_AES_256_GCM_SHA384`
- `TLS_CHACHA20_POLY1305_SHA256`
- `TLS_AES_128_GCM_SHA256`

### Certificate Rotation

Certificates should be rotated before expiration:

```bash
# Check certificate expiration
openssl x509 -in server.crt -noout -enddate

# Automated rotation with cert-manager (Kubernetes)
# See DEPLOYMENT.md for cert-manager configuration
```

---

## Authentication

### JWT Authentication

JWT tokens are used for API authentication.

**Token Structure**:
```json
{
  "header": {
    "alg": "HS256",
    "typ": "JWT"
  },
  "payload": {
    "sub": "user_01H5XXXXXX",
    "org_id": "org_01H5XXXXXX",
    "roles": ["admin"],
    "iat": 1234567890,
    "exp": 1234571490
  }
}
```

**Server Validation**:
```rust
pub struct JwtConfig {
    /// Secret for HS256 signing (production: use RS256 with key rotation)
    pub secret: SecretString,
    /// Token expiration (default: 1 hour)
    pub token_ttl: Duration,
    /// Refresh token expiration (default: 7 days)
    pub refresh_ttl: Duration,
    /// Issuer claim
    pub issuer: String,
    /// Audience claim
    pub audience: Vec<String>,
}
```

### API Key Authentication

API keys are used for service accounts and automation.

**Key Format**: `vdb_<environment>_<random_bytes>`

Example: `vdb_prod_a1b2c3d4e5f6g7h8i9j0...`

**Storage**:
- Keys are hashed (BLAKE3) before storage
- Only the hash is stored, never the raw key
- Keys can be scoped to specific operations

```rust
pub struct ApiKey {
    /// Key ID (public, used for lookup)
    pub id: ApiKeyId,
    /// Key hash (BLAKE3)
    pub key_hash: Hash,
    /// Organization this key belongs to
    pub org_id: OrgId,
    /// Allowed scopes
    pub scopes: Vec<Scope>,
    /// Expiration (optional)
    pub expires_at: Option<Timestamp>,
    /// Created timestamp
    pub created_at: Timestamp,
}

pub enum Scope {
    Read,
    Write,
    Admin,
    Query,
    Export,
}
```

### WebAuthn/Passkeys

For user authentication, WebAuthn provides phishing-resistant credentials.

**Supported Authenticators**:
- Platform authenticators (Touch ID, Windows Hello, Face ID)
- Security keys (YubiKey, SoloKey)
- Cross-platform (passkeys synced via iCloud/Google)

**Configuration**:
```rust
pub struct WebAuthnConfig {
    /// Relying party ID (your domain)
    pub rp_id: String,
    /// Relying party origin
    pub rp_origin: Url,
    /// Relying party name (displayed to user)
    pub rp_name: String,
    /// Allowed authenticator attachments
    pub authenticator_attachment: Option<AuthenticatorAttachment>,
    /// Require user verification (PIN/biometric)
    pub user_verification: UserVerificationRequirement,
}
```

### OAuth Providers

Supported OAuth providers:
- GitHub (implemented)
- Google (planned)
- Microsoft (planned)
- Custom OIDC (planned)

**OAuth Flow**:
1. User clicks "Sign in with GitHub"
2. Redirect to provider with PKCE challenge
3. Provider redirects back with authorization code
4. Exchange code for tokens
5. Fetch user profile
6. Create/link local user account
7. Issue JWT session token

```rust
pub struct OAuthConfig {
    /// Provider identifier
    pub provider: OAuthProvider,
    /// Client ID (public)
    pub client_id: String,
    /// Client secret (secure storage)
    pub client_secret: SecretString,
    /// Redirect URI after auth
    pub redirect_uri: Url,
    /// Requested scopes
    pub scopes: Vec<String>,
}

pub enum OAuthProvider {
    GitHub,
    Google,
    Microsoft,
    Custom { issuer: Url },
}
```

---

## Authorization (RBAC)

### Role Hierarchy

```
Owner
  └── Admin
        └── Member
              └── Viewer
```

### Permissions by Role

| Permission | Owner | Admin | Member | Viewer |
|------------|-------|-------|--------|--------|
| View data | Yes | Yes | Yes | Yes |
| Query data | Yes | Yes | Yes | Yes |
| Create streams | Yes | Yes | Yes | No |
| Append events | Yes | Yes | Yes | No |
| Delete streams | Yes | Yes | No | No |
| Manage users | Yes | Yes | No | No |
| Manage roles | Yes | No | No | No |
| Manage billing | Yes | No | No | No |
| Delete org | Yes | No | No | No |

### RBAC Implementation

```rust
pub struct Permission {
    pub resource: Resource,
    pub action: Action,
}

pub enum Resource {
    Organization(OrgId),
    Cluster(ClusterId),
    Stream(StreamId),
    User(UserId),
}

pub enum Action {
    Create,
    Read,
    Update,
    Delete,
    Admin,
}

pub fn check_permission(
    user: &User,
    org: &Organization,
    permission: &Permission,
) -> bool {
    let role = org.get_user_role(user.id);
    role.has_permission(permission)
}
```

### Resource-Level Permissions

Beyond organization-level RBAC, resources can have fine-grained permissions:

```rust
pub struct ResourcePermission {
    /// The resource
    pub resource_id: ResourceId,
    /// The principal (user or service account)
    pub principal_id: PrincipalId,
    /// Allowed actions
    pub actions: Vec<Action>,
    /// Granted by
    pub granted_by: UserId,
    /// When granted
    pub granted_at: Timestamp,
}
```

---

## Tenant Isolation

### Data Isolation

Each tenant's data is completely isolated:

```
┌─────────────────────────────────────────────────────────────────┐
│                      Craton Instance                           │
│  ┌─────────────────────────────────────────────────────────────┐│
│  │ Tenant A                                                     ││
│  │  ├── data/tenant_a/                                         ││
│  │  ├── Keys: KEK_A → DEK_A1, DEK_A2...                        ││
│  │  └── Streams: patients, visits, billing                     ││
│  └─────────────────────────────────────────────────────────────┘│
│  ┌─────────────────────────────────────────────────────────────┐│
│  │ Tenant B                                                     ││
│  │  ├── data/tenant_b/                                         ││
│  │  ├── Keys: KEK_B → DEK_B1, DEK_B2...                        ││
│  │  └── Streams: orders, inventory                              ││
│  └─────────────────────────────────────────────────────────────┘│
└─────────────────────────────────────────────────────────────────┘
```

### Isolation Guarantees

1. **Storage Isolation**: Each tenant has separate storage files
2. **Key Isolation**: Each tenant has unique encryption keys
3. **Query Isolation**: Queries cannot cross tenant boundaries
4. **Network Isolation**: NATS streams are tenant-scoped

### Tenant Context Propagation

Every request carries tenant context that is validated:

```rust
pub struct TenantContext {
    /// The authenticated tenant
    pub tenant_id: TenantId,
    /// The authenticated user
    pub user_id: UserId,
    /// User's role in this tenant
    pub role: Role,
    /// Request trace ID
    pub trace_id: TraceId,
}

impl TenantContext {
    /// Validate that an operation is allowed for this tenant
    pub fn validate_access(&self, resource: &Resource) -> Result<(), AccessDenied> {
        if resource.tenant_id() != self.tenant_id {
            return Err(AccessDenied::CrossTenantAccess);
        }
        Ok(())
    }
}
```

---

## Audit Logging

### Audit Events

All security-relevant events are logged:

```rust
pub enum AuditEvent {
    // Authentication events
    LoginSuccess { user_id: UserId, method: AuthMethod },
    LoginFailure { identifier: String, reason: String },
    Logout { user_id: UserId },
    SessionExpired { session_id: SessionId },

    // Authorization events
    PermissionGranted { user_id: UserId, permission: Permission },
    PermissionDenied { user_id: UserId, permission: Permission },
    RoleChanged { user_id: UserId, old_role: Role, new_role: Role },

    // Data access events
    QueryExecuted { user_id: UserId, query: String, rows_returned: u64 },
    DataExported { user_id: UserId, scope: ExportScope },
    StreamCreated { user_id: UserId, stream_id: StreamId },
    StreamDeleted { user_id: UserId, stream_id: StreamId },

    // Administrative events
    UserCreated { admin_id: UserId, user_id: UserId },
    UserDeleted { admin_id: UserId, user_id: UserId },
    ApiKeyCreated { user_id: UserId, key_id: ApiKeyId },
    ApiKeyRevoked { user_id: UserId, key_id: ApiKeyId },
}
```

### Audit Log Storage

Audit logs are stored in Craton itself, benefiting from:
- Immutable append-only storage
- Cryptographic hash chain
- Tamper-evident checkpoints
- Signed exports

```rust
pub struct AuditRecord {
    /// When the event occurred
    pub timestamp: Timestamp,
    /// The event
    pub event: AuditEvent,
    /// Tenant context
    pub tenant_id: TenantId,
    /// Source IP (if applicable)
    pub source_ip: Option<IpAddr>,
    /// Request trace ID
    pub trace_id: TraceId,
}
```

### Audit Log Retention

Default retention policies:
- Authentication events: 2 years
- Data access events: 7 years
- Administrative events: 10 years

Retention is configurable per compliance requirement:
- HIPAA: 6 years
- SOX: 7 years
- GDPR: Varies by purpose

---

## Security Hardening

### Network Security

1. **Firewall Rules**:
   ```
   # Allow only necessary ports
   - 5432/tcp (Craton protocol, TLS required)
   - 8080/tcp (Platform HTTP, behind ingress)
   - 9090/tcp (Metrics, internal only)
   ```

2. **Network Policies** (Kubernetes):
   ```yaml
   apiVersion: networking.k8s.io/v1
   kind: NetworkPolicy
   metadata:
     name: craton-server
   spec:
     podSelector:
       matchLabels:
         app: craton-server
     policyTypes:
       - Ingress
       - Egress
     ingress:
       - from:
           - podSelector:
               matchLabels:
                 app: platform-app
         ports:
           - port: 5432
     egress:
       - to:
           - podSelector:
               matchLabels:
                 app: nats
         ports:
           - port: 4222
   ```

### Container Security

1. **Non-root user**: Run as non-root user
2. **Read-only filesystem**: Mount root as read-only
3. **No new privileges**: Prevent privilege escalation
4. **Resource limits**: Set memory and CPU limits

```yaml
securityContext:
  runAsNonRoot: true
  runAsUser: 1000
  readOnlyRootFilesystem: true
  allowPrivilegeEscalation: false
  capabilities:
    drop:
      - ALL
```

### Secret Management

1. **Never commit secrets**: Use environment variables or secret managers
2. **Rotate regularly**: Rotate keys at least quarterly
3. **Audit access**: Log all secret access
4. **Use secret managers**: HashiCorp Vault, AWS Secrets Manager, etc.

```rust
// Example: Secret management trait
pub trait SecretProvider: Send + Sync {
    fn get_secret(&self, key: &str) -> Result<SecretString>;
    fn rotate_secret(&self, key: &str) -> Result<()>;
}

// Implementations
pub struct EnvSecretProvider;
pub struct VaultSecretProvider { client: VaultClient }
pub struct AwsSecretsProvider { client: SecretsManagerClient }
```

### Rate Limiting

Protect against abuse with rate limiting:

```rust
pub struct RateLimitConfig {
    /// Requests per window for unauthenticated requests
    pub anonymous_rps: u32,
    /// Requests per window for authenticated requests
    pub authenticated_rps: u32,
    /// Requests per window for auth endpoints specifically
    pub auth_rps: u32,
    /// Window duration
    pub window: Duration,
    /// Burst allowance
    pub burst: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            anonymous_rps: 10,
            authenticated_rps: 1000,
            auth_rps: 5,  // Strict limit on auth attempts
            window: Duration::from_secs(60),
            burst: 10,
        }
    }
}
```

---

## Incident Response

### Security Incident Levels

| Level | Description | Response Time | Examples |
|-------|-------------|---------------|----------|
| P1 | Critical | 15 minutes | Data breach, system compromise |
| P2 | High | 1 hour | Auth bypass, privilege escalation |
| P3 | Medium | 4 hours | Suspicious activity, policy violation |
| P4 | Low | 24 hours | Minor vulnerability, audit finding |

### Incident Response Procedure

1. **Detection**: Automated alerts or manual report
2. **Triage**: Assess secraton and impact
3. **Containment**: Isolate affected systems
4. **Investigation**: Analyze logs and evidence
5. **Remediation**: Fix the vulnerability
6. **Recovery**: Restore normal operations
7. **Post-mortem**: Document lessons learned

### Emergency Contacts

Configure emergency contacts in your deployment:

```yaml
# security-contacts.yaml
contacts:
  - name: Security Team
    email: security@example.com
    phone: +1-555-SECURITY
    pagerduty: PXXXXXX
  - name: On-Call Engineer
    pagerduty: PXXXXXX
```

### Revocation Procedures

**Revoke User Access**:
```bash
# Immediate session revocation
craton-admin user revoke-sessions --user-id user_01H5XXXXXX

# Disable user account
craton-admin user disable --user-id user_01H5XXXXXX
```

**Revoke API Key**:
```bash
craton-admin apikey revoke --key-id key_01H5XXXXXX
```

**Rotate Secrets**:
```bash
# Rotate JWT signing key (invalidates all tokens)
craton-admin secrets rotate --type jwt

# Rotate encryption keys (transparent re-encryption)
craton-admin secrets rotate --type dek --tenant-id tenant_01H5XXXXXX
```

---

## Related Documentation

- [COMPLIANCE.md](COMPLIANCE.md) - Compliance requirements
- [DEPLOYMENT.md](DEPLOYMENT.md) - Deployment guide
- [OPERATIONS.md](OPERATIONS.md) - Operations runbook
- [BUG_BOUNTY.md](BUG_BOUNTY.md) - Security research program
