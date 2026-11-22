// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Shared state storage: one small SQL layer with two backends.
//!
//! The control plane keeps users, tokens, operations and the audit trail in SQLite
//! by default. With `ZORVIA_DATABASE_URL=postgres://...` the same stores run on a
//! PostgreSQL server instead, which lets several API replicas share them (a revoked
//! session is revoked everywhere, one operation queue, one audit trail). The stores
//! keep their logic and write SQL with `?1, ?2` placeholders; this layer runs it on
//! either backend. See docs/POSTGRES.md.

pub mod sql;

pub use sql::{Backend, Row, Value};

/// `ZORVIA_DATABASE_URL`, when set and not empty.
pub fn database_url() -> Option<String> {
    std::env::var("ZORVIA_DATABASE_URL")
        .ok()
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty())
}
