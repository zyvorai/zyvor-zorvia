// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! `POST /api/v1/vm-lint`: lint a KubeVirt `VirtualMachine` manifest for day-2
//! pitfalls (see `crate::gitops::lint`). Pure computation on the request body;
//! it reads nothing from the cluster and changes nothing. `GET /api/v1/vm-lint/rules`
//! lists the rule catalogue.

use super::*;
use crate::gitops::lint::{lint_vm, RULES};

pub async fn vm_lint_handler(Json(manifest): Json<serde_json::Value>) -> impl IntoResponse {
    if !manifest.is_object() {
        let (st, j) = err_json(
            400,
            "INVALID",
            "body must be a KubeVirt VirtualMachine object",
        );
        return (st, j).into_response();
    }
    Json(lint_vm(&manifest)).into_response()
}

pub async fn vm_lint_rules_handler() -> impl IntoResponse {
    Json(serde_json::json!({ "rules": RULES }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    async fn body(r: axum::response::Response) -> (u16, serde_json::Value) {
        let st = r.status().as_u16();
        let b = axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap();
        (
            st,
            serde_json::from_slice(&b).unwrap_or(serde_json::Value::Null),
        )
    }

    #[tokio::test]
    async fn non_object_bodies_are_rejected() {
        for v in [json!(null), json!(3), json!("x"), json!([1])] {
            let (st, _) = body(vm_lint_handler(Json(v)).await.into_response()).await;
            assert_eq!(st, 400);
        }
    }

    #[tokio::test]
    async fn lints_a_manifest_and_lists_rules() {
        let m = json!({"spec":{"template":{"spec":{"domain":{"devices":{"interfaces":[{"name":"d","bridge":{}}]}},"networks":[{"name":"d","pod":{}}]}}}});
        let (st, v) = body(vm_lint_handler(Json(m)).await.into_response()).await;
        assert_eq!(st, 200);
        assert_eq!(v["live_migration_possible"], false);
        let (st, v) = body(vm_lint_rules_handler().await.into_response()).await;
        assert_eq!(st, 200);
        assert_eq!(v["rules"].as_array().unwrap().len(), RULES.len());
    }
}
