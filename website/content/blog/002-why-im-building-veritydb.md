---
title: "Why I'm Building Craton"
slug: "why-im-building-craton"
date: 2026-01-22
excerpt: "The story behind Craton starts with a clinic management system, a lot of compliance headaches, and a search for the database that didn't exist."
author_name: "Jared Reyes"
author_avatar: "/public/images/jared-avatar.jpg"
---

# Why I'm Building Craton

This is a personal story. It's about building something I wish existed, because I spent too long trying to make do with tools that weren't designed for the problem I was solving.

## It Started with Notebar

A few years ago, I started building Notebar—a clinic management system for healthcare providers. The goal was straightforward: help clinics manage patient records, appointments, and documentation in a way that was actually pleasant to use.

What I didn't anticipate was how much of my time would be spent on *compliance infrastructure* rather than *application features*.

Healthcare data isn't like other data. You can't just throw it in Postgres and call it a day. Every access needs to be logged. Every modification needs an audit trail. Data needs to be retained for specific periods. You need to prove—not just claim—that patient records haven't been tampered with.

I found myself building a lot of this from scratch:

- Audit logging tables that captured who changed what, when, and why
- Hash chains to detect unauthorized modifications
- Retention policies with legal hold support
- Multi-tenant isolation that could withstand security audits

None of this was my core product. It was all infrastructure tax.

## The Search for Something Better

I started looking for databases that could handle this out of the box. Surely someone had solved this problem?

**SQLite** was appealing for its simplicity and local-first nature. I loved the idea of fast reads without network roundtrips. But SQLite doesn't replicate. For a multi-user clinic application, that's a non-starter.

**SQLite + LiteFS** seemed promising. Fly.io's LiteFS gives you read replicas of SQLite across regions. But it's primary-based replication—writes still have to go to a single node. And the audit trail problem remains unsolved.

**NATS + Jetstream** caught my attention next. Here was a distributed log I could build on. Jetstream gives you persistent, ordered streams. I experimented with an Event Sourcing / CQRS architecture: write events to Jetstream, project them into local SQLite databases for fast reads.

This actually worked reasonably well. I could:
- Write events to a distributed, durable log
- Replay events to rebuild state
- Keep SQLite projections on each node for sub-millisecond reads

But I was still building all the compliance infrastructure myself. Event sourcing gives you a natural audit trail, but it doesn't give you:
- Cryptographic tamper evidence
- Verifiable reads with proofs
- Multi-tenant isolation at the data layer
- Retention policies and legal holds
- Compliance-ready export formats

I was essentially building a database on top of a message broker on top of a database. The complexity was mounting.

## What I Actually Needed

After months of experimentation, I had a clear picture of what I wanted:

1. **Append-only log as the source of truth** — Like Jetstream, but with cryptographic hash chaining so tampering is mathematically detectable.

2. **Fast local reads** — Like SQLite. I wanted sub-millisecond queries without network roundtrips for read-heavy workloads.

3. **Replication built in** — Not bolted on. Writes should be durably replicated before acknowledgment.

4. **Compliance as a first-class feature** — Audit trails, data classification, retention policies, verifiable exports. Not afterthoughts.

5. **Multi-tenant by design** — Tenant isolation that's structural, not just "we check the tenant_id in every query."

This database didn't exist. So I decided to build it.

## Enter Craton

Craton is my answer to the problem I couldn't solve with existing tools.

The core architecture is simple: **all data is an immutable, ordered log. All state is a derived view.**

Every write appends to a hash-chained log. Every record cryptographically links to its predecessor. The log is the system of record—everything else (tables, indexes, query results) is derived from it and can be rebuilt from scratch.

This gives us compliance properties for free:

- **Tamper evidence**: Modify any record, and the hash chain breaks
- **Complete audit trail**: The log *is* the audit trail
- **Point-in-time queries**: Replay the log to any position
- **Verifiable reads**: Every query result comes with a cryptographic proof

For local reads, we use **projections**—derived views that are rebuilt from the log. These live on each node. You get SQLite-like read performance without sacrificing the distributed log as the source of truth.

## Why Open Source?

I could have built this as a proprietary product. But I believe compliance infrastructure should be auditable by the people who depend on it.

If you're storing patient records or financial data in a system, you should be able to verify how that system works. "Trust us" isn't compliance. Transparency is.

The core of Craton is Apache 2.0 licensed. The code is public. You can audit it, contribute to it, or run it yourself.

Commercial offerings exist for organizations that want managed operations, additional compliance tooling, and support. But the open source version is complete and correct—nothing is artificially limited.

## Still Early

Craton is under active development. I'm building in public because I believe the design deserves scrutiny before it becomes production infrastructure.

If you've faced similar challenges—building compliance features when you just wanted to build an application—I'd love to hear from you. The best software comes from understanding real problems.

Star the [GitHub repo](https://github.com/craton-db/craton) to follow along, or reach out with your use cases. Let's build something worth trusting.
