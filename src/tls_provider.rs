// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! reqwest 0.13 is built with `rustls-no-provider` (so aws-lc-rs and its C
//! toolchain stay out of the tree) and panics when it builds a client without
//! a process-default rustls crypto provider. Install ring once at startup.

/// Install the ring crypto provider as the process default (idempotent).
pub fn install_default() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}
