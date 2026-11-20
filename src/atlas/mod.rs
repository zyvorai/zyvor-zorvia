// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Server-side integration with Atlas, the Zyvor-suite storage control plane.
//!
//! Zorvia keeps the Atlas service-account bearer token on the server; the web
//! UI talks only to Zorvia's `/api/v1/atlas/*` endpoints. Read-only inventory
//! plus one write path (volume creation) — see `docs/ATLAS_INTEGRATION.md`
//! for what's deliberately not proxied yet.

mod client;
pub mod models;

pub use client::{Client, Config, Error};
