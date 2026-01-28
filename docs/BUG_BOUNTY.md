# Craton Bug Bounty Program

This document describes Craton's security research program, inspired by TigerBeetle's consensus challenge. Our goal is to build a database worthy of trust by inviting the security community to find vulnerabilities before they reach production.

---

## Table of Contents

1. [Program Overview](#program-overview)
2. [Staged Rollout](#staged-rollout)
3. [Scope and Rules](#scope-and-rules)
4. [Invariants](#invariants)
5. [Reward Structure](#reward-structure)
6. [Submission Process](#submission-process)
7. [Hall of Fame](#hall-of-fame)

---

## Program Overview

### Philosophy

Craton is a compliance-first database for regulated industries. We believe that:

1. **Security through obscurity doesn't work** - Our code is open, our invariants are documented
2. **Adversarial testing finds real bugs** - DST catches implementation bugs, bounties catch design flaws
3. **Incentives matter** - Fair rewards attract serious researchers

### Inspiration

- **TigerBeetle Consensus Challenge**: $20,000 bounty for violating consensus safety
- **FoundationDB**: 18 months of simulation testing before production
- **Jepsen Analyses**: Rigorous distributed systems testing methodology

### Timeline

| Stage | Availability | Prerequisite |
|-------|--------------|--------------|
| Stage 1: Foundation | Phase 2 complete | Crypto + storage tested |
| Stage 2: Consensus | Phase 3 complete | VOPR validated |
| Stage 3: Full | Phase 8 complete | All components tested |

---

## Staged Rollout

### Stage 1: Foundation Bounty

**Scope**: `craton-crypto`, `craton-storage` crates only

**Target Vulnerabilities**:

| Category | Description | Secraton |
|----------|-------------|----------|
| Hash chain bypass | Modify log entry without detection | Critical |
| Signature forgery | Create valid signature without private key | Critical |
| Nonce reuse | Demonstrate nonce collision in encryption | Critical |
| Storage corruption | Cause data loss or corruption | High |
| Checksum bypass | Modify record without CRC detection | Medium |

**Out of Scope**:
- Consensus protocol (not yet implemented)
- Wire protocol (not yet implemented)
- Denial of service without data impact

**Bounty Range**: $500 - $5,000

### Stage 2: Consensus Bounty

**Scope**: Add `craton-vsr`, `craton-sim` crates

**Target Vulnerabilities**:

| Category | Description | Secraton |
|----------|-------------|----------|
| Safety violation | Committed data lost or changed | Critical ($20,000) |
| Linearizability | Client observes impossible ordering | Critical |
| Log divergence | Replicas disagree on committed entries | Critical |
| View change exploit | Corrupt state during leader election | High |
| Liveness failure | Cluster stops making progress | Medium |

**The Consensus Challenge**:

We will offer a **$20,000 bounty** (TigerBeetle-style) for anyone who can:

1. Start a Craton cluster
2. Perform operations that get acknowledged
3. Demonstrate that acknowledged data is lost or corrupted
4. Provide a reproducible test case (seed for VOPR or script)

**Bounty Range**: $1,000 - $20,000

### Stage 3: Full Bounty

**Scope**: All crates, full system

**Target Vulnerabilities**:

| Category | Description | Secraton |
|----------|-------------|----------|
| MVCC isolation | See uncommitted or inconsistent data | Critical |
| Authentication bypass | Access data without valid credentials | Critical |
| Encryption bypass | Read encrypted data without key | Critical |
| Data exfiltration | Extract data beyond token scope | High |
| Audit log tampering | Modify audit trail undetected | High |
| Token forgery | Create valid access token without authority | High |
| Query injection | Execute unauthorized operations | Medium |

**Bounty Range**: $500 - $50,000

---

## Scope and Rules

### In Scope

| Component | Crate | Stage |
|-----------|-------|-------|
| Hash chains | `craton-crypto` | 1+ |
| Signatures | `craton-crypto` | 1+ |
| Encryption | `craton-crypto` | 1+ |
| Append-only log | `craton-storage` | 1+ |
| VSR consensus | `craton-vsr` | 2+ |
| Simulation harness | `craton-sim` | 2+ |
| B+tree projections | `craton-store` | 3+ |
| MVCC | `craton-store` | 3+ |
| Query execution | `craton-query` | 3+ |
| Wire protocol | `craton-wire` | 3+ |
| Access tokens | `craton-sharing` | 3+ |

### Out of Scope

| Category | Reason |
|----------|--------|
| Dependencies (unless misconfigured) | Report upstream |
| Social engineering | Not a code vulnerability |
| Physical attacks | Requires hardware access |
| DoS without data impact | Low secraton |
| Bugs in test code only | Not production code |
| Known issues in TODO/FIXME | Already tracked |

### Rules of Engagement

1. **No production systems** - Test only against local instances
2. **No data exfiltration** - Don't extract real user data
3. **Responsible disclosure** - Report before publishing
4. **One bug per report** - Separate issues for separate bugs
5. **Reproducible** - Include steps, seed, or script

### Legal Safe Harbor

We will not pursue legal action against researchers who:
- Follow the rules above
- Report in good faith
- Don't cause harm to real users
- Allow reasonable time for fixes

---

## Invariants

These are the properties Craton must maintain. Violations are bugs.

### Cryptographic Invariants

```
INV-CRYPTO-1: Hash Chain Integrity
  For all records R[i] where i > 0:
    R[i].prev_hash == hash(R[i-1])

INV-CRYPTO-2: Signature Validity
  For all signed checkpoints C:
    verify(C.public_key, C.data, C.signature) == true

INV-CRYPTO-3: Nonce Uniqueness
  For all encrypted records E1, E2 with same key K:
    E1.nonce != E2.nonce

INV-CRYPTO-4: Key Isolation
  For all tenants T1, T2 where T1 != T2:
    T1.dek != T2.dek
```

### Consensus Invariants

```
INV-VSR-1: Agreement
  For all replicas R1, R2 and position P:
    if committed(R1, P) and committed(R2, P):
      R1.log[P] == R2.log[P]

INV-VSR-2: Durability
  For all acknowledged writes W:
    W survives any f failures (where cluster has 2f+1 nodes)

INV-VSR-3: Linearizability
  Client observations are consistent with some total ordering
  of all operations

INV-VSR-4: View Change Safety
  View changes preserve all committed operations
```

### Storage Invariants

```
INV-STORE-1: Append-Only
  Once written, log entries are never modified

INV-STORE-2: Sequential Positions
  For all records R[i], R[i+1]:
    R[i+1].position == R[i].position + 1

INV-STORE-3: Checksum Validity
  For all records R:
    R.checksum == crc32(R.data)

INV-STORE-4: MVCC Visibility
  Queries at position P see exactly the state as of P
```

### Projection Invariants

```
INV-PROJ-1: Derivability
  Projection state == Apply(initial_state, log[0..applied_position])

INV-PROJ-2: Consistency
  projection.applied_position <= log.committed_position

INV-PROJ-3: Determinism
  Same log + same initial state == same projection state
```

---

## Reward Structure

### Secraton Levels

| Secraton | Impact | Example | Range |
|----------|--------|---------|-------|
| **Critical** | Data loss, integrity violation, auth bypass | Consensus safety violation, key extraction | $5,000 - $50,000 |
| **High** | Significant security impact | Audit log tampering, token forgery | $2,000 - $10,000 |
| **Medium** | Limited impact, requires conditions | Information disclosure, DoS with recovery | $500 - $2,000 |
| **Low** | Minimal impact | Non-sensitive info leak, minor issues | $100 - $500 |

### Bonus Multipliers

| Factor | Multiplier | Description |
|--------|------------|-------------|
| Novel attack | 1.5x | Previously unknown attack class |
| Affects production | 1.5x | Demonstrated in realistic scenario |
| With fix | 1.25x | Includes working patch |
| Elegant reproduction | 1.1x | Minimal, clear test case |

### Special Bounties

| Challenge | Reward | Criteria |
|-----------|--------|----------|
| **Consensus Challenge** | $20,000 | Violate VSR safety with reproducible case |
| **Encryption Challenge** | $10,000 | Extract plaintext without key |
| **Audit Challenge** | $5,000 | Modify audit log undetected |

---

## Submission Process

### How to Report

1. **Email**: security@craton.io (PGP key available)
2. **HackerOne**: [hackerone.com/craton](https://hackerone.com/craton) (when available)

### What to Include

```markdown
## Summary
[One-line description of the vulnerability]

## Secraton
[Critical/High/Medium/Low]

## Affected Component
[Crate name and version]

## Reproduction Steps
1. [Step 1]
2. [Step 2]
3. [Expected vs actual behavior]

## VOPR Seed (if applicable)
[Seed that reproduces the issue]

## Impact
[What an attacker could do]

## Suggested Fix (optional)
[How to remediate]
```

### Response Timeline

| Stage | Timeline |
|-------|----------|
| Acknowledgment | 24 hours |
| Initial assessment | 72 hours |
| Secraton confirmation | 1 week |
| Fix development | 2-4 weeks |
| Bounty payment | Within 30 days of fix |
| Public disclosure | After fix released |

### Coordinated Disclosure

We request 90 days before public disclosure. We will:
- Credit you in release notes (unless you prefer anonymity)
- Add you to the Hall of Fame
- Coordinate disclosure timing

---

## Hall of Fame

*This section will list security researchers who have responsibly disclosed vulnerabilities.*

### 2025

| Researcher | Finding | Secraton | Bounty |
|------------|---------|----------|--------|
| *Be the first!* | - | - | - |

---

## Resources

### For Researchers

- [ARCHITECTURE.md](ARCHITECTURE.md) - System design
- [TESTING.md](TESTING.md) - VOPR and simulation testing
- [CRATONICS.md](CRATONICS.md) - Coding philosophy (explains our invariants)
- Source code: [github.com/craton/craton](https://github.com/craton/craton)

### Related Programs

- [TigerBeetle Consensus Challenge](https://github.com/tigerbeetle/viewstamped-replication-made-famous)
- [Jepsen Analyses](https://jepsen.io/analyses)
- [Google Project Zero](https://googleprojectzero.blogspot.com/)

---

## Contact

- **Security issues**: security@craton.io
- **Program questions**: bounty@craton.io
- **General inquiries**: hello@craton.io

PGP Key Fingerprint: `[To be added]`

---

*This program is subject to change. Check this document for the latest terms.*
