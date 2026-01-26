# syntax=docker/dockerfile:1

# VerityDB Website Container
# Multi-stage build for minimal image size

# Stage 1: Build the Rust binary
FROM --platform=linux/amd64 rust:1.88-bookworm AS builder

# Build version for cache busting (git commit hash passed at build time)
ARG BUILD_VERSION=unknown

WORKDIR /app

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

# Copy assets needed at build time (CSS minification + Askama templates)
COPY public ./public
COPY templates ./templates

# Build the release binary with cache-busting version
ENV BUILD_VERSION=${BUILD_VERSION}
RUN cargo build --release -p vdb-site

# Stage 2: Runtime image
FROM --platform=linux/amd64 debian:bookworm-slim

# Install CA certificates for HTTPS requests (if needed in future)
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the binary from builder
COPY --from=builder /app/target/release/vdb-site ./

# Copy static assets and templates
COPY public ./public
COPY content ./content
COPY templates ./templates

# Environment configuration
ENV HOST=0.0.0.0
ENV PORT=3000
ENV RUST_LOG=info

# Expose the port
EXPOSE 3000

# Run the server
CMD ["./vdb-site"]
