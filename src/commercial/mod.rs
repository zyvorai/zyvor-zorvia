// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Commercial workflows: service catalog, quote requests, contracts and
//! coverage. Records purchased services and support entitlements only.
//!
//! Nothing here gates VM functionality: an expired or missing contract
//! changes *commercial service eligibility* and nothing else. Commercial
//! tables live in their own SQLite file (`ZORVIA_COMMERCIAL_DB`), separate
//! from VM operational state and from the auth database.

pub mod billing;
pub mod capacity;
pub mod catalog;
pub mod diagnostics;
pub mod managed;
pub mod model;
pub mod redact;
pub mod services;
pub mod store;
pub mod support;
pub mod timers;

pub use model::*;
pub use store::CommercialStore;

use std::sync::{Arc, OnceLock};

static STORE: OnceLock<Result<Arc<CommercialStore>, String>> = OnceLock::new();

/// Process-wide store, opened on first use. An open failure is remembered
/// and surfaced as "unavailable" rather than taking the server down.
pub fn store() -> Result<Arc<CommercialStore>, String> {
    STORE
        .get_or_init(|| {
            CommercialStore::from_env()
                .map(Arc::new)
                .map_err(|e| format!("commercial store unavailable: {e:#}"))
        })
        .clone()
}
