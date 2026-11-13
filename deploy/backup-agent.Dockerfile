# Off-cluster backup Job entrypoint image. The agent only reads disk images
# from read-only PVC mounts and talks HTTPS to the object store, so it needs no
# privileges and no extra packages beyond CA certificates.
FROM rust:1.99-slim-bookworm@sha256:24707e70a4f1e23d9a7b25202ec43126d28a52294b96aad5c670df1fd1f1d400 AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
RUN cargo build --release --locked --no-default-features --features backup-agent --bin backup-agent \
    && strip target/release/backup-agent

FROM debian:bookworm-slim@sha256:99c0b25c46d0214d5a2091092e7908907dcaecd6a962cde0ac700acee222b6f3

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/backup-agent /usr/local/bin/backup-agent

# UID 107 owns CDI-provisioned disk images (qemu); the Job also sets it.
USER 107
ENTRYPOINT ["backup-agent"]
