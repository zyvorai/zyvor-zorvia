// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Server-side integration with the Kryton Windows virtualization control plane.
//!
//! Zorvia authenticates the browser and keeps the Kryton bearer token on the
//! server. The web UI talks only to Zorvia's `/api/v1/kryton/*` endpoints.

mod client;
pub mod models;

pub use client::{Client, Config, Error};
