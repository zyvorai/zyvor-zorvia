//! Commercial catalog, quote requests, contracts and coverage.
//!
//! Backed by `crate::commercial` (its own SQLite file). Organization
//! isolation is enforced in the store via `Caller`; routes under
//! `/v1/commercial/admin/` additionally require `users.admin` (see
//! `required_permission`). Nothing here touches VMs or gates their APIs.

use super::*;
use crate::commercial::store::{
    Caller, CommercialError, ImportBundle, QuoteInput, QuoteRequestInput,
};
use crate::commercial::{catalog, ContractStatus, Entitlement, PaymentStatus};
use axum::extract::Json as AxumJson;
use axum::response::Response;
use serde_json::json;

pub(super) type Auth = Option<axum::Extension<crate::api::auth::AuthIdentity>>;

pub(super) fn caller_of(auth: &Auth) -> Result<Caller, Box<Response>> {
    let Some(axum::Extension(id)) = auth else {
        let (st, j) = err_json(401, "UNAUTHORIZED", "Authentication required");
        return Err(Box::new((st, j).into_response()));
    };
    Ok(Caller {
        username: id.username.clone().unwrap_or_else(|| "api-token".into()),
        admin: id.has_permission(crate::api::auth::ApiPermission::UsersAdmin),
    })
}

pub(super) fn store() -> Result<std::sync::Arc<crate::commercial::CommercialStore>, Box<Response>> {
    crate::commercial::store().map_err(|e| {
        log::error!("{e}");
        let (st, j) = err_json(
            503,
            "COMMERCIAL_UNAVAILABLE",
            "Commercial records are unavailable on this server",
        );
        Box::new((st, j).into_response())
    })
}

pub(super) fn fail(e: CommercialError) -> Response {
    let (st, j) = match e {
        CommercialError::NotFound => err_json(404, "NOT_FOUND", "Not found"),
        CommercialError::Invalid(m) => err_json(400, "INVALID_REQUEST", &m),
        CommercialError::Conflict(m) => err_json(409, "CONFLICT", &m),
        CommercialError::Forbidden(m) => err_json(403, "FORBIDDEN", &m),
        CommercialError::Internal(e) => {
            log::error!("commercial store error: {e:#}");
            err_json(500, "INTERNAL", "Internal error")
        }
    };
    (st, j).into_response()
}

/// Run `f` with the store and caller; map every failure to a response.
pub(super) fn with<T: Serialize>(
    auth: &Auth,
    status: StatusCode,
    f: impl FnOnce(&crate::commercial::CommercialStore, &Caller) -> Result<T, CommercialError>,
) -> Response {
    let caller = match caller_of(auth) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    let store = match store() {
        Ok(s) => s,
        Err(r) => return *r,
    };
    match f(&store, &caller) {
        Ok(v) => (status, Json(v)).into_response(),
        Err(e) => fail(e),
    }
}

// ── public ───────────────────────────────────────────────────────

/// Public: what can be purchased, who is responsible for what. Carries no
/// prices, and states plainly which integrations are not configured.
pub async fn commercial_catalog_handler() -> impl IntoResponse {
    Json(json!({
        "offerings": catalog::OFFERINGS,
        "pricing": "Prices are set per quote by authorized commercial administrators; none are published here.",
        "community_note": "All existing functionality remains available without a subscription under the Zyvor Production License v1.0. Contracts describe purchased services and support coverage only.",
        "integrations": {
            "notifications": { "available": false, "reason": "No notification integration is configured; requests are recorded but nothing is sent." },
            "payments": { "available": false, "reason": "No payment provider is configured; no charges are made." },
            "remote_management": { "available": false, "reason": "Remote operation is disabled and is never enabled by purchasing a contract." }
        }
    }))
}

// ── customer-facing (org-scoped) ─────────────────────────────────

pub async fn commercial_quote_requests_list(auth: Auth) -> Response {
    with(&auth, StatusCode::OK, |s, c| s.list_quote_requests(c))
}

pub async fn commercial_quote_request_create(
    auth: Auth,
    AxumJson(body): AxumJson<QuoteRequestInput>,
) -> Response {
    // Records the request only. Sending a notification would require a
    // configured integration and is not done here.
    with(&auth, StatusCode::CREATED, |s, c| {
        s.create_quote_request(c, body)
    })
}

pub async fn commercial_contracts_list(auth: Auth) -> Response {
    with(&auth, StatusCode::OK, |s, c| {
        // Persist expiry as commercial bookkeeping; failure is non-fatal
        // because effective_status is computed on read regardless.
        if let Err(e) = s.sweep_expired() {
            log::warn!("commercial expiry sweep failed: {e}");
        }
        s.list_contracts(c)
    })
}

pub async fn commercial_contract_get(auth: Auth, Path(id): Path<String>) -> Response {
    with(&auth, StatusCode::OK, |s, c| s.get_contract(c, &id))
}

pub async fn commercial_contract_history(auth: Auth, Path(id): Path<String>) -> Response {
    with(&auth, StatusCode::OK, |s, c| s.history(c, &id))
}

pub async fn commercial_contract_accept(auth: Auth, Path(id): Path<String>) -> Response {
    with(&auth, StatusCode::OK, |s, c| s.accept(c, &id))
}

#[derive(Debug, Deserialize)]
pub struct CoverageQuery {
    pub cluster: Option<String>,
}

pub async fn commercial_coverage(auth: Auth, Query(q): Query<CoverageQuery>) -> Response {
    with(&auth, StatusCode::OK, |s, c| {
        if let Err(e) = s.sweep_expired() {
            log::warn!("commercial expiry sweep failed: {e}");
        }
        s.coverage(c, q.cluster.as_deref())
    })
}

// ── administration (users.admin) ─────────────────────────────────

pub async fn commercial_admin_orgs_list(auth: Auth) -> Response {
    with(&auth, StatusCode::OK, |s, c| s.list_orgs(c))
}

#[derive(Debug, Deserialize)]
pub struct CreateOrgBody {
    pub name: String,
    pub first_member: Option<String>,
}

pub async fn commercial_admin_org_create(
    auth: Auth,
    AxumJson(body): AxumJson<CreateOrgBody>,
) -> Response {
    with(&auth, StatusCode::CREATED, |s, _| {
        s.create_org(&body.name, body.first_member.as_deref())
    })
}

#[derive(Debug, Deserialize)]
pub struct MemberBody {
    pub username: String,
}

pub async fn commercial_admin_member_add(
    auth: Auth,
    Path(org_id): Path<String>,
    AxumJson(body): AxumJson<MemberBody>,
) -> Response {
    with(&auth, StatusCode::OK, |s, _| {
        s.add_member(&org_id, &body.username)?;
        Ok(json!({ "org_id": org_id, "username": body.username }))
    })
}

pub async fn commercial_admin_member_remove(
    auth: Auth,
    Path((org_id, username)): Path<(String, String)>,
) -> Response {
    with(&auth, StatusCode::OK, |s, _| {
        s.remove_member(&org_id, &username)?;
        Ok(json!({ "removed": true }))
    })
}

pub async fn commercial_admin_quote_generate(
    auth: Auth,
    Path(request_id): Path<String>,
    AxumJson(body): AxumJson<QuoteInput>,
) -> Response {
    with(&auth, StatusCode::CREATED, |s, c| {
        s.generate_quote(&c.username, &request_id, body)
    })
}

pub async fn commercial_admin_entitlement_put(
    auth: Auth,
    Path(id): Path<String>,
    AxumJson(body): AxumJson<Entitlement>,
) -> Response {
    with(&auth, StatusCode::OK, |s, c| {
        s.update_entitlement(&c.username, &id, body)
    })
}

#[derive(Debug, Deserialize)]
pub struct TransitionBody {
    pub to: String,
}

pub async fn commercial_admin_transition(
    auth: Auth,
    Path(id): Path<String>,
    AxumJson(body): AxumJson<TransitionBody>,
) -> Response {
    with(&auth, StatusCode::OK, |s, c| {
        let to = ContractStatus::parse(&body.to)
            .ok_or_else(|| CommercialError::Invalid("unknown contract status".into()))?;
        s.transition(&c.username, &id, to)
    })
}

#[derive(Debug, Deserialize)]
pub struct PaymentStatusBody {
    pub status: String,
}

pub async fn commercial_admin_payment_status(
    auth: Auth,
    Path(id): Path<String>,
    AxumJson(body): AxumJson<PaymentStatusBody>,
) -> Response {
    with(&auth, StatusCode::OK, |s, c| {
        let st = PaymentStatus::parse(&body.status)
            .ok_or_else(|| CommercialError::Invalid("unknown payment status".into()))?;
        s.set_payment_status(&c.username, &id, st)
    })
}

pub async fn commercial_admin_import(
    auth: Auth,
    AxumJson(body): AxumJson<ImportBundle>,
) -> Response {
    with(&auth, StatusCode::CREATED, |s, c| {
        s.import_contract(&c.username, body)
    })
}
