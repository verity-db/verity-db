---
title: "Veritaserum: The Craton Coding Philosophy"
slug: "veritaserum-our-coding-philosophy"
date: 2026-01-21
excerpt: "Why I choose correctness over convenience, and how these coding principles serve the regulated industries that depend on Craton."
author_name: "Jared Reyes"
author_avatar: "/public/images/jared-avatar.jpg"
---

# Veritaserum: The Craton Coding Philosophy

I named the coding philosophy after Veritaserum—the truth serum from Harry Potter. The name isn't just whimsy. It captures what I'm trying to achieve: code that is transparent, honest, and leaves no room for deception.

Craton is a compliance-first database. Users store healthcare records, financial transactions, legal documents. When they ask "did this data change?", they need an answer they can stake their business on.

That responsibility flows down to every line of code.

## Correctness Over Convenience

Most databases optimize for flexibility and developer convenience. That's fine for prototyping. It's not fine for systems of record.

I take a different approach: **correctness by construction**. Instead of writing code that's probably correct and hoping tests catch the bugs, I structure the code so that entire categories of bugs are impossible.

This means:
- Slower initial development
- More thought required upfront
- Less "move fast and break things"

It also means:
- Fewer production incidents
- Bugs that are caught by the compiler, not by customers
- Code that auditors can actually verify

## Make Illegal States Unrepresentable

This is the most important principle in the codebase. If something shouldn't happen, make it *impossible*, not just checked.

Consider a simple example. Many codebases track state with booleans:

```rust
struct Request {
    is_authenticated: bool,
    is_admin: bool,
}
```

What happens when `is_admin` is true but `is_authenticated` is false? The code "works," but it represents a state that should never exist. Somewhere, there's probably an `if` statement that checks both flags. Maybe.

I make this impossible to represent:

```rust
enum RequestAuth {
    Anonymous,
    Authenticated(UserId),
    Admin(AdminId),
}
```

Now the compiler enforces the invariant. An admin is always a specific user. Anonymous requests have no user ID. The impossible state is *unrepresentable*.

This pattern scales up. I use newtypes instead of primitives, so you can't accidentally pass a `TenantId` where a `StreamId` is expected. I encode state machines in types, so you can't call `.commit()` on a transaction that hasn't been prepared.

The goal is to make the code path from "valid state" to "valid state" the only path the compiler allows.

## Functional Core, Imperative Shell

Craton's kernel is a pure, deterministic state machine. You give it a command and the current state, it returns the new state and a list of effects to execute.

The kernel:
- Takes no I/O
- Uses no clocks
- Has no randomness
- Has no side effects

All of that lives in the "shell"—the outer layer that executes effects, handles network I/O, and provides the current time when asked.

Why does this matter for compliance?

**Deterministic replay.** Given the same starting state and the same sequence of commands, you always get the same result. This makes debugging possible—replay the log and watch the bug happen. It makes testing possible—property-based tests can explore the state space without mocking.

**Simulation testing.** I can run thousands of simulated nodes in a single process, because the kernel is just a pure function. No network mocks. No filesystem stubs. Just function calls.

**Auditability.** When someone asks "what would have happened if...", I can answer definitively by replaying with different inputs.

## Parse, Don't Validate

Data validation is a common source of bugs. You check that an input is valid, then pass the raw input to another function, which might forget to check.

I parse once at system boundaries and use typed representations throughout.

```rust
// This is a TenantId - guaranteed valid by construction
struct TenantId(u64);

impl TenantId {
    pub fn parse(s: &str) -> Result<Self, ParseError> {
        let id: u64 = s.parse()?;
        if id == 0 {
            return Err(ParseError::ZeroId);
        }
        Ok(TenantId(id))
    }
}

// Internal functions receive TenantId, not &str
// They cannot receive invalid input
fn load_tenant(id: TenantId) -> Tenant { ... }
```

Once data crosses the trust boundary and becomes a typed value, it's never validated again. The type *is* the proof of validity.

## Assertion Density

Every function in the codebase has at least two assertions: one precondition and one postcondition.

Assertions serve two purposes:

**Living documentation.** The assertions tell you what the function expects and guarantees. They're comments that the runtime verifies.

**Early failure.** When invariants are violated, I want to fail immediately and loudly, not subtly corrupt data and fail hours later.

I write assertions in pairs—one at the write site, one at the read site:

```rust
// When writing
fn write_record(storage: &mut Storage, record: &Record) {
    let checksum = crc32(&record.data);
    storage.write_u32(checksum);
    storage.write(&record.data);
}

// When reading
fn read_record(storage: &Storage, offset: Offset) -> Record {
    let stored_checksum = storage.read_u32(offset);
    let data = storage.read_bytes(offset + 4);
    let computed_checksum = crc32(&data);

    // Paired assertion: verify what write_record wrote
    assert_eq!(
        stored_checksum, computed_checksum,
        "record corruption at offset {:?}", offset
    );

    Record { data }
}
```

## Explicit Control Flow

The codebase has no recursion. Every loop has explicit bounds. Control flow is visible.

Why no recursion? Stack overflow risks. Unbounded resource consumption. Difficulty reasoning about worst-case behavior.

Instead, I use explicit iteration with bounds:

```rust
fn traverse(root: &Node, max_depth: usize) {
    let mut stack = vec![(root, 0)];

    while let Some((node, depth)) = stack.pop() {
        assert!(depth <= max_depth, "max depth exceeded");
        process(node);

        for child in &node.children {
            stack.push((child, depth + 1));
        }
    }
}
```

The depth limit is explicit. The stack is visible. Resource usage is bounded.

## Minimal Dependencies

Every dependency is trust extended. Every crate pulled in becomes part of the trusted computing base.

I evaluate dependencies carefully:

- **Can I implement this in under 200 lines?** If so, I probably should.
- **Is it well-maintained?** Active development, responsive maintainers, semver discipline.
- **Has it been audited?** Security-critical code should have third-party review.
- **What does it pull in?** Transitive dependencies count.

I prefer the standard library. I vendor when it makes sense. I question every `cargo add`.

## The Philosophy Serves the Mission

These aren't arbitrary rules. Each principle exists because Craton is compliance infrastructure.

When a hospital stores patient records in Craton, they're trusting the system with data that could affect care decisions. When a financial institution stores transaction records, they're trusting it with data that regulators will audit.

That trust demands code that is:
- **Correct** — Bugs in compliance infrastructure become legal liability
- **Auditable** — Regulators and security teams need to verify claims
- **Predictable** — Surprise behavior in a database is never acceptable

I write code that I would trust with my own medical records.

---

*The full Veritaserum coding philosophy is documented in the [repository](https://github.com/craton-db/craton). Scrutiny is welcome.*
