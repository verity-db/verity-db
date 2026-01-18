# Contributing to Verity

Thank you for your interest in contributing to Verity.

Verity is a correctness-first system of record designed for regulated
environments. As a result, we place a high bar on design clarity,
determinism, and explainability.

Please read this document carefully before contributing.

---

## Philosophy

Verity follows a few non-negotiable principles:

- The append-only log is the system of record.
- All state is derived and must be replayable.
- Correctness and explainability take precedence over convenience.
- Performance optimizations must preserve determinism.
- Compliance is structural, not procedural.

Contributions are evaluated primarily on how well they preserve these
invariants.

---

## What we welcome

We welcome contributions in the following areas:

- correctness fixes
- performance improvements with clear reasoning
- documentation and design clarification
- tests (especially failure and recovery scenarios)
- tooling that improves observability or verification
- SDK ergonomics (without weakening guarantees)

Small, well-scoped changes are preferred over large feature additions.

---

## What we are cautious about

We are cautious about:

- expanding the query language without strong justification
- introducing background processes with unpredictable behavior
- adding implicit behavior or hidden side effects
- introducing dependencies that reduce deployability or auditability

Feature requests that significantly expand scope may be deferred or
redirected to external integrations.

---

## Design-first contributions

For non-trivial changes, please open a design discussion before
submitting a pull request.

A good design proposal should include:

- the problem being solved
- relevant invariants
- failure modes and recovery behavior
- how correctness is preserved
- how the change can be tested

Code without an accompanying explanation is unlikely to be accepted.

---

## Code style and expectations

- Favor explicitness over cleverness.
- Avoid hidden global state.
- Prefer bounded work and predictable behavior.
- Keep APIs minimal and intention-revealing.
- Tests should be deterministic and reproducible.

---

## Licensing and contributions

Verity is licensed under the Apache License 2.0.

By submitting a contribution, you agree that:

- your contribution is licensed under the Apache 2.0 license
- you have the right to submit the contribution
- you grant the project the right to use, modify, and distribute it

If you are contributing on behalf of an employer, ensure you have
appropriate permission to do so.

We may introduce a Contributor License Agreement (CLA) in the future
to simplify commercial distribution.

---

## Security and correctness issues

If you believe you have found a security or correctness issue that
could affect data integrity or compliance, please do not open a public
issue.

Instead, contact the maintainers privately.

---

## Getting started

- Read the architecture documentation
- Review the system invariants
- Explore the examples directory
- Start with small, well-contained changes

If you are unsure where to begin, open an issue and ask.

---

Thank you for helping make Verity correct, explainable, and trustworthy.
