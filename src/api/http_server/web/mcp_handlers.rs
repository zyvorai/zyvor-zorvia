//! `POST /api/mcp`: the Model Context Protocol endpoint.
//!
//! Tool calls are replayed through the REST router in-process with the caller's own credential,
//! so authentication, RBAC, namespace tenancy, audit and webhooks apply exactly as they do for a
//! direct REST call. See `crate::mcp` for the protocol and the tool table.

use super::*;
use crate::mcp::{BoxFuture, DispatchResult, Dispatcher, McpConfig};
use axum::body::{Body, Bytes};
use axum::extract::{Json as AxumJson, Request};
use axum::response::Response;
use axum::{Extension, Router};
use tower::ServiceExt;

/// The REST router tool calls are replayed through: `/api/*` behind `auth_middleware`, and
/// without the MCP route itself, so a tool call can never recurse into MCP.
#[derive(Clone)]
pub struct McpInner(pub Router);

/// Largest REST response a tool call will read back.
const MAX_REST_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

struct RouterDispatcher {
    router: Router,
    /// The caller's credential headers (`x-api-key` and/or `authorization`), forwarded as-is.
    credentials: Vec<(header::HeaderName, header::HeaderValue)>,
}

impl RouterDispatcher {
    fn new(router: Router, headers: &HeaderMap) -> Self {
        let credentials = [
            header::HeaderName::from_static("x-api-key"),
            header::AUTHORIZATION,
        ]
        .into_iter()
        .filter_map(|name| headers.get(&name).map(|v| (name, v.clone())))
        .collect();
        Self {
            router,
            credentials,
        }
    }
}

impl Dispatcher for RouterDispatcher {
    fn dispatch<'a>(
        &'a self,
        method: &'static str,
        path: String,
        body: Option<serde_json::Value>,
    ) -> BoxFuture<'a, DispatchResult> {
        Box::pin(async move {
            let mut builder = Request::builder().method(method).uri(format!("/api{path}"));
            for (name, value) in &self.credentials {
                builder = builder.header(name, value);
            }
            let request_body = match body {
                Some(b) => {
                    builder = builder.header(header::CONTENT_TYPE, "application/json");
                    Body::from(b.to_string())
                }
                None => Body::empty(),
            };
            let request = match builder.body(request_body) {
                Ok(r) => r,
                Err(e) => {
                    return DispatchResult {
                        status: 400,
                        body: serde_json::json!({ "message": format!("bad request: {e}") }),
                    }
                }
            };
            let response = match self.router.clone().oneshot(request).await {
                Ok(r) => r,
                Err(never) => match never {},
            };
            let status = response.status().as_u16();
            let bytes = axum::body::to_bytes(response.into_body(), MAX_REST_RESPONSE_BYTES)
                .await
                .unwrap_or_default();
            let body = if bytes.is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::from_slice(&bytes).unwrap_or_else(|_| {
                    serde_json::Value::String(String::from_utf8_lossy(&bytes).into_owned())
                })
            };
            DispatchResult { status, body }
        })
    }
}

pub async fn mcp_post(
    Extension(inner): Extension<McpInner>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !crate::mcp::enabled_from_env() {
        // Off by default: indistinguishable from a route that does not exist.
        return err_json(404, "NOT_FOUND", "MCP is not enabled on this server").into_response();
    }
    let payload: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return AxumJson(crate::mcp::parse_error()).into_response(),
    };
    let dispatcher = RouterDispatcher::new(inner.0, &headers);
    match crate::mcp::handle_payload(&payload, &dispatcher, &McpConfig::from_env()).await {
        Some(reply) => AxumJson(reply).into_response(),
        // Only notifications/responses were sent: acknowledge without a body.
        None => StatusCode::ACCEPTED.into_response(),
    }
}

/// Streamable HTTP lets a server decline the optional GET event stream with 405.
pub async fn mcp_get() -> Response {
    StatusCode::METHOD_NOT_ALLOWED.into_response()
}
