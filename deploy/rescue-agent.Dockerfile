# Rescue mode's Job entrypoint image. GuestKit's disk-mount path shells out
# to losetup/qemu-nbd/mount/chroot (see src/kube/rescue.rs and
# docs/RESCUE.md) -- this needs the same package set GuestKit's own worker
# image ships (qemu-utils, util-linux, kmod), and the container must run
# with SYS_ADMIN + SYS_RESOURCE (set in the Job's pod spec, not here).
FROM rust:1.99-slim-bookworm@sha256:2c3a22f0a5533ea2dd5a16627bc841228151faa2d4de2644ac9987e4a2f1f2fa AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
RUN cargo build --release --locked --no-default-features --features rescue-agent --bin rescue-agent \
    && strip target/release/rescue-agent

FROM debian:bookworm-slim@sha256:99c0b25c46d0214d5a2091092e7908907dcaecd6a962cde0ac700acee222b6f3

RUN apt-get update && apt-get install -y --no-install-recommends \
    qemu-utils \
    util-linux \
    kmod \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/rescue-agent /usr/local/bin/rescue-agent

# Must run as root: losetup/qemu-nbd/mount/chroot need real privilege, same
# as GuestKit's own worker image (crates/guestkit-worker/Dockerfile in
# ../guestkit) and its k8s/daemonset.yaml's SYS_ADMIN+SYS_RESOURCE pod spec.
ENTRYPOINT ["rescue-agent"]
