// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! `GET /api/v1/adopt/report` — the same read-only adoption report as
//! `zorvia adopt`, built from the shared logic in `crate::adopt`.

use super::*;
use crate::adopt::{build_report, VmSummary};

#[derive(Debug, Deserialize)]
pub struct AdoptQuery {
    pub namespace: Option<String>,
    #[serde(default)]
    pub all_namespaces: bool,
}

pub async fn adopt_report_handler(
    State(state): State<SharedState>,
    Query(query): Query<AdoptQuery>,
) -> impl IntoResponse {
    let s = state.read().await;
    let namespace = query
        .namespace
        .clone()
        .unwrap_or_else(|| s.namespace.clone());
    let client = s.client();
    drop(s);

    let vms = if query.all_namespaces {
        client.list_all_vms().await
    } else {
        client.list_vms(&namespace).await
    };

    match vms {
        Ok(vms) => {
            let summaries: Vec<VmSummary> = vms.iter().map(VmSummary::from_vm).collect();
            let report = build_report(&summaries);
            let ctx = req_ctx(HttpMethod::GET, "/api/v1/adopt/report");
            ok_json(&ApiResponse::success(&report, &ctx.request_id))
        }
        Err(e) => err_json(500, "INTERNAL_ERROR", &sanitize_error(&e)),
    }
}
