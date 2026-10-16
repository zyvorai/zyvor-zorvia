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
    handle_mcp_post(
        crate::mcp::enabled_from_env(),
        &McpConfig::from_env(),
        inner.0,
        &headers,
        &body,
    )
    .await
}

/// The endpoint's logic with its switches passed in, so tests need not touch process-wide env.
async fn handle_mcp_post(
    enabled: bool,
    cfg: &McpConfig,
    router: Router,
    headers: &HeaderMap,
    body: &[u8],
) -> Response {
    if !enabled {
        // Off by default: indistinguishable from a route that does not exist.
        return err_json(404, "NOT_FOUND", "MCP is not enabled on this server").into_response();
    }
    let payload: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return AxumJson(crate::mcp::parse_error()).into_response(),
    };
    let dispatcher = RouterDispatcher::new(router, headers);
    match crate::mcp::handle_payload(&payload, &dispatcher, cfg).await {
        Some(reply) => AxumJson(reply).into_response(),
        // Only notifications/responses were sent: acknowledge without a body.
        None => StatusCode::ACCEPTED.into_response(),
    }
}

/// Streamable HTTP lets a server decline the optional GET event stream with 405.
pub async fn mcp_get() -> Response {
    StatusCode::METHOD_NOT_ALLOWED.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::{get, post};
    use serde_json::{json, Value};

    /// Stands in for the real `/api/*` router: echoes what it receives so the tests can see exactly
    /// what the replay sent.
    fn stub() -> Router {
        Router::new()
            .route(
                "/api/vms/{name}/start",
                post(|headers: HeaderMap, axum::extract::Path(name): axum::extract::Path<String>| async move {
                    let h = |k: &str| headers.get(k).and_then(|v| v.to_str().ok()).map(str::to_string);
                    AxumJson(json!({
                        "vm": name,
                        "x-api-key": h("x-api-key"),
                        "authorization": h("authorization"),
                        "cookie": h("cookie"),
                        "origin": h("origin"),
                        "x-forwarded-for": h("x-forwarded-for"),
                        "content-type": h("content-type"),
                    }))
                }),
            )
            .route(
                "/api/vms",
                post(|headers: HeaderMap, body: Bytes| async move {
                    let ct = headers
                        .get("content-type")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    (
                        StatusCode::CREATED,
                        AxumJson(json!({ "content-type": ct, "body": String::from_utf8_lossy(&body) })),
                    )
                }),
            )
            .route(
                "/api/vms/gone",
                get(|| async {
                    (StatusCode::NOT_FOUND, AxumJson(json!({"error": {"message": "VM not found"}})))
                }),
            )
            .route("/api/vms/text", get(|| async { "plain text, not json" }))
            .route(
                "/api/vms/deny",
                get(|| async {
                    (
                        StatusCode::FORBIDDEN,
                        AxumJson(json!({"error": "FORBIDDEN", "message": "Missing permission: vm.power"})),
                    )
                }),
            )
            .route("/api/vms/empty", axum::routing::delete(|| async { StatusCode::NO_CONTENT }))
    }

    fn creds(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                v.parse().unwrap(),
            );
        }
        h
    }

    fn rpc(method: &str, params: Value) -> Vec<u8> {
        serde_json::to_vec(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
            .unwrap()
    }

    async fn post_mcp(enabled: bool, headers: &HeaderMap, body: &[u8]) -> (StatusCode, Value) {
        let resp = handle_mcp_post(enabled, &McpConfig::default(), stub(), headers, body).await;
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
            .await
            .unwrap();
        let v = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, v)
    }

    /// The text of a tool result, for assertions.
    fn tool_text(v: &Value) -> String {
        v["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }

    #[tokio::test]
    async fn replay_forwards_only_the_callers_credentials() {
        let headers = creds(&[
            ("x-api-key", "tok-123"),
            ("authorization", "Bearer jwt-abc"),
            ("cookie", "session=steal"),
            ("origin", "https://evil.example"),
            ("x-forwarded-for", "9.9.9.9"),
        ]);
        let body = rpc(
            "tools/call",
            json!({"name":"start_vm","arguments":{"name":"web-1"}}),
        );
        let (status, v) = post_mcp(true, &headers, &body).await;
        assert_eq!(status, StatusCode::OK);
        let echoed: Value = serde_json::from_str(&tool_text(&v)).unwrap();
        assert_eq!(echoed["vm"], "web-1");
        assert_eq!(echoed["x-api-key"], "tok-123");
        assert_eq!(echoed["authorization"], "Bearer jwt-abc");
        // Nothing else the client sent may ride along into the inner request.
        assert!(echoed["cookie"].is_null());
        assert!(echoed["origin"].is_null());
        assert!(echoed["x-forwarded-for"].is_null());
        assert!(
            echoed["content-type"].is_null(),
            "bodyless POST must not claim a body"
        );
    }

    #[tokio::test]
    async fn replay_sends_json_body_with_content_type() {
        let body = rpc(
            "tools/call",
            json!({"name":"create_vm","arguments":{"name":"a","image":"quay.io/x:1","cpus":2,"memory":1024}}),
        );
        let (_, v) = post_mcp(true, &creds(&[("x-api-key", "k")]), &body).await;
        assert_eq!(v["result"]["isError"], false, "{v}");
        let echoed: Value = serde_json::from_str(&tool_text(&v)).unwrap();
        assert_eq!(echoed["content-type"], "application/json");
        let sent: Value = serde_json::from_str(echoed["body"].as_str().unwrap()).unwrap();
        assert_eq!(
            sent,
            json!({"name":"a","image":"quay.io/x:1","cpus":2,"memory":1024})
        );
    }

    #[tokio::test]
    async fn rest_status_codes_become_tool_errors() {
        let h = creds(&[("x-api-key", "k")]);
        // 404 with a nested error message
        let (_, v) = post_mcp(
            true,
            &h,
            &rpc(
                "tools/call",
                json!({"name":"get_vm","arguments":{"name":"gone"}}),
            ),
        )
        .await;
        assert_eq!(v["result"]["isError"], true);
        let t = tool_text(&v);
        assert!(t.contains("404") && t.contains("VM not found"), "{t}");
        // 403 keeps the permission name so the model can tell the user what is missing
        let (_, v) = post_mcp(
            true,
            &h,
            &rpc(
                "tools/call",
                json!({"name":"get_vm","arguments":{"name":"deny"}}),
            ),
        )
        .await;
        let t = tool_text(&v);
        assert!(
            t.contains("403") && t.contains("vm.power") && t.contains("lacks permission"),
            "{t}"
        );
    }

    #[tokio::test]
    async fn non_json_and_empty_rest_bodies_are_handled() {
        let h = creds(&[("x-api-key", "k")]);
        let (_, v) = post_mcp(
            true,
            &h,
            &rpc(
                "tools/call",
                json!({"name":"get_vm","arguments":{"name":"text"}}),
            ),
        )
        .await;
        assert_eq!(v["result"]["isError"], false);
        assert!(tool_text(&v).contains("plain text, not json"));
        let (_, v) = post_mcp(
            true,
            &h,
            &rpc(
                "tools/call",
                json!({"name":"delete_vm","arguments":{"name":"empty","confirm":true}}),
            ),
        )
        .await;
        assert_eq!(tool_text(&v), "OK (204)");
    }

    #[tokio::test]
    async fn disabled_server_answers_404_even_for_valid_requests() {
        let (status, v) = post_mcp(
            false,
            &creds(&[("x-api-key", "k")]),
            &rpc("tools/list", json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(v.to_string().contains("NOT_FOUND"));
    }

    #[tokio::test]
    async fn malformed_and_notification_payloads() {
        let h = HeaderMap::new();
        // Not JSON at all: JSON-RPC parse error (HTTP 200 with an error object)
        let (status, v) = post_mcp(true, &h, b"{not json").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["error"]["code"], -32700);
        // Empty body is also a parse error, not a panic
        let (_, v) = post_mcp(true, &h, b"").await;
        assert_eq!(v["error"]["code"], -32700);
        // A notification gets 202 and no body
        let note = br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        let (status, v) = post_mcp(true, &h, note).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert!(v.is_null());
        // A JSON scalar is an invalid request
        let (_, v) = post_mcp(true, &h, b"42").await;
        assert_eq!(v["error"]["code"], -32600);
    }

    #[tokio::test]
    async fn tool_calls_cannot_reach_the_mcp_route_or_other_apis() {
        // The replay router is the stub here; whatever the arguments say, the request path is
        // always one of the table's fixed templates, never caller-chosen.
        let h = creds(&[("x-api-key", "k")]);
        for evil in ["../mcp", "mcp/..", "v1/users", "a/../b", "%2e%2e"] {
            let (_, v) = post_mcp(
                true,
                &h,
                &rpc(
                    "tools/call",
                    json!({"name":"get_vm","arguments":{"name":evil}}),
                ),
            )
            .await;
            assert_eq!(v["result"]["isError"], true, "{evil}");
            assert!(
                tool_text(&v).starts_with("Invalid arguments"),
                "{evil}: {}",
                tool_text(&v)
            );
        }
    }

    #[tokio::test]
    async fn get_is_not_allowed() {
        assert_eq!(mcp_get().await.status(), StatusCode::METHOD_NOT_ALLOWED);
    }
}
