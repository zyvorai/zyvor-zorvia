// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

use super::SharedState;
use crate::kryton::models::{CreateMachineRequest, GoldenStartRequest, SnapshotRequest};
use crate::kryton::{Client, Error as KrytonError};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(super) struct ProjectQuery {
    project: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct MachineListQuery {
    project: Option<String>,
    limit: Option<u16>,
    cursor: Option<String>,
}

fn error_response(error: KrytonError) -> Response {
    match error {
        KrytonError::Upstream {
            status,
            code,
            message,
            hint,
        } => {
            let status = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
            (
                status,
                Json(serde_json::json!({
                    "success": false,
                    "error": {
                        "code": code.unwrap_or_else(|| "KRYTON_UPSTREAM".into()),
                        "message": message,
                        "hint": hint,
                    }
                })),
            )
                .into_response()
        }
        KrytonError::MissingProject => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "success": false,
                "error": {"code": "KRYTON_PROJECT_REQUIRED", "message": error.to_string()}
            })),
        )
            .into_response(),
        KrytonError::Transport(_) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "success": false,
                "error": {
                    "code": "KRYTON_UNREACHABLE",
                    "message": "Zorvia could not reach the configured Kryton control plane"
                }
            })),
        )
            .into_response(),
    }
}

async fn client(state: &SharedState) -> Result<Client, Box<Response>> {
    state.read().await.kryton.clone().ok_or_else(|| {
        Box::new(
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "success": false,
                    "error": {
                        "code": "KRYTON_DISABLED",
                        "message": "Kryton integration is disabled; configure KRYTON_URL on the Zorvia server"
                    }
                })),
            )
                .into_response(),
        )
    })
}

pub(super) async fn kryton_status(State(state): State<SharedState>) -> Response {
    let c = match client(&state).await {
        Ok(c) => c,
        Err(_) => {
            return Json(serde_json::json!({
                "enabled": false,
                "connected": false,
                "project": null,
            }))
            .into_response()
        }
    };
    match c.ready().await {
        Ok(ready) => Json(serde_json::json!({
            "enabled": true,
            "connected": true,
            "project": c.configured_project(),
            "ready": ready,
        }))
        .into_response(),
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "enabled": true,
                "connected": false,
                "project": c.configured_project(),
                "error": error.to_string(),
            })),
        )
            .into_response(),
    }
}

pub(super) async fn kryton_capabilities(State(state): State<SharedState>) -> Response {
    let c = match client(&state).await {
        Ok(v) => v,
        Err(r) => return *r,
    };
    match c.capabilities().await {
        Ok(v) => Json(v).into_response(),
        Err(e) => error_response(e),
    }
}

pub(super) async fn kryton_doctor(State(state): State<SharedState>) -> Response {
    let c = match client(&state).await {
        Ok(v) => v,
        Err(r) => return *r,
    };
    match c.doctor().await {
        Ok(v) => Json(v).into_response(),
        Err(e) => error_response(e),
    }
}

pub(super) async fn kryton_images(State(state): State<SharedState>) -> Response {
    let c = match client(&state).await {
        Ok(v) => v,
        Err(r) => return *r,
    };
    match c.images().await {
        Ok(v) => Json(v).into_response(),
        Err(e) => error_response(e),
    }
}

pub(super) async fn kryton_summary(
    State(state): State<SharedState>,
    Query(query): Query<ProjectQuery>,
) -> Response {
    let c = match client(&state).await {
        Ok(v) => v,
        Err(r) => return *r,
    };
    match c.summary(query.project.as_deref()).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => error_response(e),
    }
}

pub(super) async fn kryton_list_machines(
    State(state): State<SharedState>,
    Query(query): Query<MachineListQuery>,
) -> Response {
    let c = match client(&state).await {
        Ok(v) => v,
        Err(r) => return *r,
    };
    match c
        .machines(
            query.project.as_deref(),
            query.limit,
            query.cursor.as_deref(),
        )
        .await
    {
        Ok(v) => Json(v).into_response(),
        Err(e) => error_response(e),
    }
}

pub(super) async fn kryton_get_machine(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Query(query): Query<ProjectQuery>,
) -> Response {
    let c = match client(&state).await {
        Ok(v) => v,
        Err(r) => return *r,
    };
    match c.machine(&id, query.project.as_deref()).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => error_response(e),
    }
}

pub(super) async fn kryton_create_machine(
    State(state): State<SharedState>,
    Json(body): Json<CreateMachineRequest>,
) -> Response {
    let c = match client(&state).await {
        Ok(v) => v,
        Err(r) => return *r,
    };
    match c.create(body).await {
        Ok(v) => (StatusCode::CREATED, Json(v)).into_response(),
        Err(e) => error_response(e),
    }
}

pub(super) async fn kryton_start_machine(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Query(query): Query<ProjectQuery>,
) -> Response {
    let c = match client(&state).await {
        Ok(v) => v,
        Err(r) => return *r,
    };
    match c.start(&id, query.project.as_deref()).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => error_response(e),
    }
}

pub(super) async fn kryton_stop_machine(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Query(query): Query<ProjectQuery>,
) -> Response {
    let c = match client(&state).await {
        Ok(v) => v,
        Err(r) => return *r,
    };
    match c.stop(&id, query.project.as_deref()).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => error_response(e),
    }
}

pub(super) async fn kryton_delete_machine(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Query(query): Query<ProjectQuery>,
) -> Response {
    let c = match client(&state).await {
        Ok(v) => v,
        Err(r) => return *r,
    };
    match c.delete(&id, query.project.as_deref()).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => error_response(e),
    }
}

pub(super) async fn kryton_snapshot_machine(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Query(query): Query<ProjectQuery>,
    Json(body): Json<SnapshotRequest>,
) -> Response {
    let c = match client(&state).await {
        Ok(v) => v,
        Err(r) => return *r,
    };
    match c.snapshot(&id, query.project.as_deref(), body.name).await {
        Ok(v) => (StatusCode::CREATED, Json(v)).into_response(),
        Err(e) => error_response(e),
    }
}

pub(super) async fn kryton_list_snapshots(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Query(query): Query<ProjectQuery>,
) -> Response {
    let c = match client(&state).await {
        Ok(v) => v,
        Err(r) => return *r,
    };
    match c.snapshots(&id, query.project.as_deref()).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => error_response(e),
    }
}

pub(super) async fn kryton_restore_snapshot(
    State(state): State<SharedState>,
    Path((id, sid)): Path<(String, String)>,
    Query(query): Query<ProjectQuery>,
) -> Response {
    let c = match client(&state).await {
        Ok(v) => v,
        Err(r) => return *r,
    };
    match c
        .restore_snapshot(&id, &sid, query.project.as_deref())
        .await
    {
        Ok(v) => (StatusCode::ACCEPTED, Json(v)).into_response(),
        Err(e) => error_response(e),
    }
}

pub(super) async fn kryton_delete_snapshot(
    State(state): State<SharedState>,
    Path((id, sid)): Path<(String, String)>,
    Query(query): Query<ProjectQuery>,
) -> Response {
    let c = match client(&state).await {
        Ok(v) => v,
        Err(r) => return *r,
    };
    match c.delete_snapshot(&id, &sid, query.project.as_deref()).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => error_response(e),
    }
}

pub(super) async fn kryton_list_golden(State(state): State<SharedState>) -> Response {
    let c = match client(&state).await {
        Ok(v) => v,
        Err(r) => return *r,
    };
    match c.golden_builds().await {
        Ok(v) => Json(v).into_response(),
        Err(e) => error_response(e),
    }
}

pub(super) async fn kryton_get_golden(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Response {
    let c = match client(&state).await {
        Ok(v) => v,
        Err(r) => return *r,
    };
    match c.golden_build(&id).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => error_response(e),
    }
}

pub(super) async fn kryton_start_golden(
    State(state): State<SharedState>,
    Query(query): Query<ProjectQuery>,
    Json(body): Json<GoldenStartRequest>,
) -> Response {
    let c = match client(&state).await {
        Ok(v) => v,
        Err(r) => return *r,
    };
    match c.start_golden(&body, query.project.as_deref()).await {
        Ok(v) => (StatusCode::ACCEPTED, Json(v)).into_response(),
        Err(e) => error_response(e),
    }
}

pub(super) async fn kryton_bootstrap_golden(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Query(query): Query<ProjectQuery>,
) -> Response {
    let c = match client(&state).await {
        Ok(v) => v,
        Err(r) => return *r,
    };
    match c.bootstrap_golden(&id, query.project.as_deref()).await {
        Ok(v) => (StatusCode::ACCEPTED, Json(v)).into_response(),
        Err(e) => error_response(e),
    }
}

pub(super) async fn kryton_golden_passport(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Response {
    let c = match client(&state).await {
        Ok(v) => v,
        Err(r) => return *r,
    };
    match c.golden_passport(&id).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => error_response(e),
    }
}
