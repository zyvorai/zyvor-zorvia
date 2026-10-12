# Multi-stage build for minimal production image
#
# Cargo.lock is lockfile format v4 (has been since 0.3.0), which needs
# Cargo 1.78+ to even parse -- `cargo build --locked` fails immediately
# on anything older, independent of the crate's own rust-version MSRV.
FROM rust:1.99-slim-bookworm@sha256:2c3a22f0a5533ea2dd5a16627bc841228151faa2d4de2644ac9987e4a2f1f2fa AS builder

# openssl-sys needs pkg-config + the OpenSSL dev headers to find the system
# OpenSSL, and rusqlite's "bundled" feature compiles SQLite's C amalgamation
# from source -- the slim base image has none of gcc/pkg-config/libssl-dev
# (GitHub's hosted runners do, which is why the plain `cargo build` CI job
# never hit this).
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Cache dependency compilation
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && echo "" > src/lib.rs \
    && cargo build --release --locked 2>/dev/null || true \
    && rm -rf src

# Build actual source
COPY src/ src/
RUN cargo build --release --locked && strip target/release/zorvia

# Runtime image
FROM debian:bookworm-slim@sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/zorvia /usr/local/bin/zorvia

RUN useradd -u 10001 -m zorvia
USER zorvia

# Health checking is handled by Kubernetes probes (livenessProbe/readinessProbe)
# targeting /api/v1/health on the running server.

ENTRYPOINT ["zorvia"]
CMD ["api", "api-serve"]
