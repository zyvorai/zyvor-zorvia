# Multi-stage build for minimal production image

# Web console: built from web/ so the released image serves the UI (ZORVIA_WEB_DIR below).
FROM node:26-bookworm-slim@sha256:28147155d47dc31abe0a46e0d12b59b2fb3d0a2dcc5dc65fce76a074180b16fe AS web
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN npm ci --legacy-peer-deps
COPY web/ ./
RUN npm run build

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
# The dummy sources above were compiled moments ago; COPY keeps the checkout's
# older mtimes, so without this cargo reuses the stub and ships a no-op binary.
RUN touch src/main.rs src/lib.rs && cargo build --release --locked && strip target/release/zorvia

# Runtime image
FROM debian:bookworm-slim@sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates openssh-client \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/zorvia /usr/local/bin/zorvia
COPY --from=web /web/dist /usr/share/zorvia/web
ENV ZORVIA_WEB_DIR=/usr/share/zorvia/web

RUN useradd -u 10001 -m zorvia
USER zorvia

# Health checking is handled by Kubernetes probes (livenessProbe/readinessProbe)
# targeting /api/v1/health on the running server.

ENTRYPOINT ["zorvia"]
CMD ["api", "api-serve"]
