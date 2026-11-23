# Rescue mode's Job entrypoint image. GuestKit's disk-mount path shells out
# to losetup/qemu-nbd/mount/chroot (see src/kube/rescue.rs and
# docs/RESCUE.md) -- this needs the same package set GuestKit's own worker
# image ships (qemu-utils, util-linux, kmod), and the container must run
# with SYS_ADMIN + SYS_RESOURCE (set in the Job's pod spec, not here).
# install-packages also bind-mounts /proc, /dev and /sys into the guest chroot
# (needs the same capabilities) and needs outbound network access from the pod.
#
# The binary builds without the server's default `web` feature; the
# lib's web-only modules are cfg-gated so no server dependencies are pulled in.
FROM rust:1.100-slim-bookworm@sha256:7c7d4eb4d05b7781187a327809466cfddf29753f6defdc3d19a0e291951b7a37 AS builder

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

FROM debian:bookworm-slim@sha256:734a72b3d6142a71993b4bc580ca01d8ca687f77a2c422d96a961d3376b179bc

RUN apt-get update && apt-get install -y --no-install-recommends \
    qemu-utils \
    util-linux \
    kmod \
    openssl \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/rescue-agent /usr/local/bin/rescue-agent

# Must run as root: losetup/qemu-nbd/mount/chroot need real privilege, same
# as GuestKit's own worker image (crates/guestkit-worker/Dockerfile in
# ../guestkit) and its k8s/daemonset.yaml's SYS_ADMIN+SYS_RESOURCE pod spec.
ENTRYPOINT ["rescue-agent"]
