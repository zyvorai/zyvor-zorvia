// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Service engagements and managed-operations records.
//!
//! Records only: nothing here reaches a cluster. Org isolation and the
//! customer-versus-desk rules live in `crate::commercial::{services,managed}`;
//! `/admin/` routes additionally require `users.admin` (see permissions.rs).

use super::commercial_handlers::{with, Auth};
use super::*;
use crate::commercial::managed::{
    IncidentUpdate, NewEnrollment, NewIncident, NewPolicy, NewRecoveryEvidence, NewReport, NewTask,
    RemoteRequest,
};
use crate::commercial::services::{MilestoneUpdate, NewEngagement, NewEvidence};
use axum::extract::Json as AxumJson;
use axum::response::Response;
use serde_json::json;

// ── engagements ──────────────────────────────────────────────────

pub async fn engagements_list(auth: Auth) -> Response {
    with(&auth, StatusCode::OK, |s, c| s.list_engagements(c))
}

pub async fn engagement_get(auth: Auth, Path(id): Path<String>) -> Response {
    with(&auth, StatusCode::OK, |s, c| {
        Ok(
            json!({ "engagement": s.get_engagement(c, &id)?, "events": s.engagement_events(c, &id)? }),
        )
    })
}

pub async fn engagement_evidence_add(
    auth: Auth,
    Path((id, key)): Path<(String, String)>,
    AxumJson(body): AxumJson<NewEvidence>,
) -> Response {
    with(&auth, StatusCode::CREATED, |s, c| {
        s.add_engagement_evidence(c, &id, &key, body)
    })
}

pub async fn engagement_accept(auth: Auth, Path(id): Path<String>) -> Response {
    with(&auth, StatusCode::OK, |s, c| s.accept_engagement(c, &id))
}

pub async fn engagement_create(auth: Auth, AxumJson(body): AxumJson<NewEngagement>) -> Response {
    with(&auth, StatusCode::CREATED, |s, c| {
        s.create_engagement(c, body)
    })
}

pub async fn engagement_milestone_update(
    auth: Auth,
    Path((id, key)): Path<(String, String)>,
    AxumJson(body): AxumJson<MilestoneUpdate>,
) -> Response {
    with(&auth, StatusCode::OK, |s, c| {
        s.update_milestone(c, &id, &key, body)
    })
}

pub async fn engagement_request_acceptance(auth: Auth, Path(id): Path<String>) -> Response {
    with(&auth, StatusCode::OK, |s, c| {
        s.request_engagement_acceptance(c, &id)
    })
}

#[derive(Debug, Deserialize)]
pub struct ReasonBody {
    pub reason: String,
}

pub async fn engagement_cancel(
    auth: Auth,
    Path(id): Path<String>,
    AxumJson(body): AxumJson<ReasonBody>,
) -> Response {
    with(&auth, StatusCode::OK, |s, c| {
        s.cancel_engagement(c, &id, &body.reason)
    })
}

// ── managed operations: customer side ───────────────────────────

pub async fn managed_enrollments_list(auth: Auth) -> Response {
    with(&auth, StatusCode::OK, |s, c| s.list_enrollments(c))
}

pub async fn managed_enroll(auth: Auth, AxumJson(body): AxumJson<NewEnrollment>) -> Response {
    with(&auth, StatusCode::CREATED, |s, c| s.enroll_cluster(c, body))
}

pub async fn managed_enrollment_get(auth: Auth, Path(id): Path<String>) -> Response {
    with(&auth, StatusCode::OK, |s, c| s.enrollment_detail(c, &id))
}

pub async fn managed_remote_operation(
    auth: Auth,
    Path(id): Path<String>,
    AxumJson(body): AxumJson<RemoteRequest>,
) -> Response {
    with(&auth, StatusCode::OK, |s, c| {
        s.set_remote_operation(c, &id, body)
    })
}

pub async fn managed_withdraw(auth: Auth, Path(id): Path<String>) -> Response {
    with(&auth, StatusCode::OK, |s, c| s.withdraw_enrollment(c, &id))
}

pub async fn managed_evidence_add(
    auth: Auth,
    Path(id): Path<String>,
    AxumJson(body): AxumJson<NewRecoveryEvidence>,
) -> Response {
    with(&auth, StatusCode::CREATED, |s, c| {
        s.add_recovery_evidence(c, &id, body)
    })
}

pub async fn managed_task_approve(auth: Auth, Path((id, tid)): Path<(String, String)>) -> Response {
    with(&auth, StatusCode::OK, |s, c| s.approve_task(c, &id, &tid))
}

pub async fn managed_policy_authorize(
    auth: Auth,
    Path((id, pid)): Path<(String, String)>,
) -> Response {
    with(&auth, StatusCode::OK, |s, c| {
        s.authorize_policy(c, &id, &pid)
    })
}

pub async fn managed_policy_revoke(
    auth: Auth,
    Path((id, pid)): Path<(String, String)>,
) -> Response {
    with(&auth, StatusCode::OK, |s, c| s.revoke_policy(c, &id, &pid))
}

// ── managed operations: service desk ────────────────────────────

#[derive(Debug, Deserialize)]
pub struct OwnerBody {
    pub owner: Option<String>,
}

pub async fn managed_owner_set(
    auth: Auth,
    Path(id): Path<String>,
    AxumJson(body): AxumJson<OwnerBody>,
) -> Response {
    with(&auth, StatusCode::OK, |s, c| {
        s.set_operational_owner(c, &id, body.owner.as_deref())
    })
}

pub async fn managed_task_propose(
    auth: Auth,
    Path(id): Path<String>,
    AxumJson(body): AxumJson<NewTask>,
) -> Response {
    with(&auth, StatusCode::CREATED, |s, c| {
        s.propose_task(c, &id, body)
    })
}

#[derive(Debug, Deserialize)]
pub struct TaskStatusBody {
    pub status: String,
}

pub async fn managed_task_status(
    auth: Auth,
    Path((id, tid)): Path<(String, String)>,
    AxumJson(body): AxumJson<TaskStatusBody>,
) -> Response {
    with(&auth, StatusCode::OK, |s, c| {
        s.set_task_status(c, &id, &tid, &body.status)
    })
}

pub async fn managed_incident_open(
    auth: Auth,
    Path(id): Path<String>,
    AxumJson(body): AxumJson<NewIncident>,
) -> Response {
    with(&auth, StatusCode::CREATED, |s, c| {
        s.open_incident(c, &id, body)
    })
}

pub async fn managed_incident_update(
    auth: Auth,
    Path((id, iid)): Path<(String, String)>,
    AxumJson(body): AxumJson<IncidentUpdate>,
) -> Response {
    with(&auth, StatusCode::OK, |s, c| {
        s.update_incident(c, &id, &iid, body)
    })
}

pub async fn managed_report_add(
    auth: Auth,
    Path(id): Path<String>,
    AxumJson(body): AxumJson<NewReport>,
) -> Response {
    with(&auth, StatusCode::CREATED, |s, c| {
        s.add_service_report(c, &id, body)
    })
}

pub async fn managed_policy_create(
    auth: Auth,
    Path(id): Path<String>,
    AxumJson(body): AxumJson<NewPolicy>,
) -> Response {
    with(&auth, StatusCode::CREATED, |s, c| {
        s.create_policy(c, &id, body)
    })
}
