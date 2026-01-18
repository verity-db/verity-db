# VerityDB Development Commands
# Install just: cargo install just
# Run `just` to see available commands

set dotenv-load := false

# Default: show available commands
default:
    @just --list

# ─────────────────────────────────────────────────────────────────────────────
# Development
# ─────────────────────────────────────────────────────────────────────────────

# Run the application in debug mode
run *args:
    cargo run -- {{args}}

# Run with release optimizations
run-release *args:
    cargo run --release -- {{args}}

# Build debug
build:
    cargo build --workspace

# Build release
build-release:
    cargo build --workspace --release

# ─────────────────────────────────────────────────────────────────────────────
# Testing
# ─────────────────────────────────────────────────────────────────────────────

# Run all tests
test:
    cargo test --workspace --all-features

# Run tests with nextest (faster, better output)
nextest:
    cargo nextest run --workspace --all-features

# Run a specific test
test-one name:
    cargo test --workspace {{name}}

# Run tests with output shown
test-verbose:
    cargo test --workspace --all-features -- --nocapture

# ─────────────────────────────────────────────────────────────────────────────
# Code Quality (mirrors CI)
# ─────────────────────────────────────────────────────────────────────────────

# Check formatting
fmt-check:
    cargo fmt --all -- --check

# Format code
fmt:
    cargo fmt --all

# Run clippy
clippy:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

# Run clippy and auto-fix
clippy-fix:
    cargo clippy --workspace --all-targets --all-features --fix --allow-dirty

# Check that docs build without warnings
doc-check:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features

# Build and open docs
doc:
    cargo doc --workspace --no-deps --all-features --open

# Check for unused dependencies
unused-deps:
    cargo machete

# ─────────────────────────────────────────────────────────────────────────────
# Security (mirrors CI)
# ─────────────────────────────────────────────────────────────────────────────

# Run security audit
audit:
    cargo audit

# Run cargo-deny checks
deny:
    cargo deny check

# Check licenses only
deny-licenses:
    cargo deny check licenses

# Check advisories only
deny-advisories:
    cargo deny check advisories

# ─────────────────────────────────────────────────────────────────────────────
# CI Simulation
# ─────────────────────────────────────────────────────────────────────────────

# Run all CI checks locally (quick version)
ci: fmt-check clippy test doc-check
    @echo "CI checks passed!"

# Run full CI checks including security
ci-full: ci unused-deps audit deny
    @echo "Full CI checks passed!"

# Pre-commit hook: fast checks before committing
pre-commit: fmt-check clippy test
    @echo "Pre-commit checks passed!"

# ─────────────────────────────────────────────────────────────────────────────
# Maintenance
# ─────────────────────────────────────────────────────────────────────────────

# Update dependencies
update:
    cargo update

# Clean build artifacts
clean:
    cargo clean

# Check MSRV (Minimum Supported Rust Version)
msrv:
    cargo +1.85 check --workspace --all-targets

# Generate code coverage report
coverage:
    cargo llvm-cov --workspace --all-features --html
    @echo "Coverage report: target/llvm-cov/html/index.html"

# Generate SBOM (Software Bill of Materials)
sbom:
    cargo cyclonedx --format json --output-prefix veritydb

# ─────────────────────────────────────────────────────────────────────────────
# Setup
# ─────────────────────────────────────────────────────────────────────────────

# Install development tools
setup:
    @echo "Installing development tools..."
    cargo install cargo-nextest cargo-audit cargo-deny cargo-machete cargo-llvm-cov
    @echo "Done! Optional: cargo install cargo-cyclonedx (for SBOM generation)"

# Install pre-commit hook
install-hooks:
    @echo '#!/bin/sh' > .git/hooks/pre-commit
    @echo 'just pre-commit' >> .git/hooks/pre-commit
    @chmod +x .git/hooks/pre-commit
    @echo "Pre-commit hook installed!"
