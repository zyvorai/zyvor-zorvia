//! MCP (Model Context Protocol) server core.
//!
//! A deliberately small, dependency-free implementation of the request/response subset of MCP
//! that AI clients need to manage VMs: `initialize`, `ping`, `tools/list` and `tools/call` over
//! JSON-RPC 2.0 (the "Streamable HTTP" transport with plain JSON responses).
//!
//! Tools are a typed facade over the existing REST API. A tool call is turned into a REST request
//! and handed to a [`Dispatcher`]; the server's real dispatcher replays it through the same router,
//! so authentication, RBAC, namespace tenancy, rate limiting, audit and webhooks are enforced by
//! the REST layer exactly as for any other client. Nothing here decides who may do what.

use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;

/// Newest protocol revision implemented. Older revisions in [`SUPPORTED_VERSIONS`] are accepted.
pub const LATEST_PROTOCOL_VERSION: &str = "2025-06-18";
const SUPPORTED_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// Cap on tool-result text so a large fleet cannot blow up a client's context window.
const MAX_RESULT_CHARS: usize = 200_000;

const INSTRUCTIONS: &str = "Zorvia manages KubeVirt virtual machines. Use list_vms / get_vm to inspect, \
start_vm / stop_vm / restart_vm for power, create_vm to provision, and the snapshot tools for recovery points. \
Destructive tools (delete_vm, delete_snapshot, revert_snapshot) do nothing unless called with confirm=true; \
describe the impact to the user and get their agreement first. Every action runs with the caller's own \
permissions and is recorded in the audit trail.";

// JSON-RPC error codes.
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// Outcome of a REST call made on behalf of a tool.
#[derive(Debug, Clone)]
pub struct DispatchResult {
    pub status: u16,
    pub body: Value,
}

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Performs the REST call behind a tool. `path` is relative to `/api` (for example `/vms/web-1/start`).
pub trait Dispatcher: Send + Sync {
    fn dispatch<'a>(
        &'a self,
        method: &'static str,
        path: String,
        body: Option<Value>,
    ) -> BoxFuture<'a, DispatchResult>;
}

/// Server-side switches.
#[derive(Debug, Clone, Default)]
pub struct McpConfig {
    /// Tool names that are not offered and cannot be called.
    pub disabled_tools: HashSet<String>,
}

impl McpConfig {
    /// `ZORVIA_MCP_DISABLE_TOOLS=delete_vm,revert_snapshot`
    pub fn from_env() -> Self {
        let disabled_tools = std::env::var("ZORVIA_MCP_DISABLE_TOOLS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        Self { disabled_tools }
    }
}

/// `ZORVIA_MCP_ENABLED=1` turns the endpoint on. It is off by default.
pub fn enabled_from_env() -> bool {
    matches!(
        std::env::var("ZORVIA_MCP_ENABLED").as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes")
    )
}

/// One MCP tool and the REST call it maps to.
pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    pub method: &'static str,
    /// REST path under `/api`; `{name}` and `{id}` are filled from the arguments of the same name.
    pub path: &'static str,
    /// Argument keys forwarded as the JSON request body (empty: no body).
    pub body_keys: &'static [&'static str],
    pub read_only: bool,
    pub destructive: bool,
    pub idempotent: bool,
    schema: fn() -> Value,
}

fn string_prop(description: &str) -> Value {
    json!({ "type": "string", "description": description })
}

fn vm_name_prop() -> Value {
    string_prop("VM name (Kubernetes name: letters, digits, '-', '.', '_').")
}

fn schema_none() -> Value {
    json!({ "type": "object", "properties": {}, "additionalProperties": false })
}

fn schema_vm() -> Value {
    json!({
        "type": "object",
        "properties": { "name": vm_name_prop() },
        "required": ["name"],
        "additionalProperties": false
    })
}

fn schema_vm_confirm() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": vm_name_prop(),
            "confirm": { "type": "boolean", "description": "Must be true. Set it only after the user has agreed to this destructive action." }
        },
        "required": ["name", "confirm"],
        "additionalProperties": false
    })
}

fn schema_create_vm() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": vm_name_prop(),
            "image": string_prop("Boot source: a container disk image (e.g. quay.io/containerdisks/ubuntu:22.04), or pvc:<name>, datavolume:<name>, blank:."),
            "cpus": { "type": "integer", "minimum": 1, "description": "Number of vCPUs." },
            "memory": { "type": "integer", "minimum": 256, "description": "Memory in MiB (minimum 256)." },
            "disk": { "type": "integer", "minimum": 1, "description": "Root disk size in GB (default 20)." },
            "start": { "type": "boolean", "description": "Start the VM after creating it." },
            "guest_os": string_prop("Guest OS hint, e.g. linux or windows."),
            "tenant": string_prop("Tenant / namespace to create the VM in, if the caller is namespace-scoped."),
            "labels": { "type": "object", "additionalProperties": { "type": "string" }, "description": "Labels to set on the VM." },
            "expose_ssh": { "type": "boolean", "description": "Expose SSH through a service." }
        },
        "required": ["name", "image", "cpus", "memory"],
        "additionalProperties": false
    })
}

fn schema_create_snapshot() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": vm_name_prop(),
            "snapshot_name": string_prop("Name for the new snapshot."),
            "description": string_prop("Optional description."),
            "snapshot_type": { "type": "string", "enum": ["Disk", "Memory"], "description": "Snapshot type (default Disk)." }
        },
        "required": ["name", "snapshot_name"],
        "additionalProperties": false
    })
}

fn schema_snapshot_confirm() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": vm_name_prop(),
            "id": string_prop("Snapshot id or name, as returned by list_snapshots."),
            "confirm": { "type": "boolean", "description": "Must be true. Set it only after the user has agreed to this destructive action." }
        },
        "required": ["name", "id", "confirm"],
        "additionalProperties": false
    })
}

/// Every tool the server can offer. This table is the whole attack surface of MCP: a tool is
/// available only if it is listed here, and the REST permission model decides who may call it.
pub static TOOLS: &[Tool] = &[
    Tool {
        name: "list_vms",
        description: "List virtual machines with their status, resources and node.",
        method: "GET",
        path: "/vms",
        body_keys: &[],
        read_only: true,
        destructive: false,
        idempotent: true,
        schema: schema_none,
    },
    Tool {
        name: "get_vm",
        description: "Get one virtual machine: spec, status, IP addresses and conditions.",
        method: "GET",
        path: "/vms/{name}",
        body_keys: &[],
        read_only: true,
        destructive: false,
        idempotent: true,
        schema: schema_vm,
    },
    Tool {
        name: "list_snapshots",
        description: "List the snapshots of a virtual machine.",
        method: "GET",
        path: "/vms/{name}/snapshots",
        body_keys: &[],
        read_only: true,
        destructive: false,
        idempotent: true,
        schema: schema_vm,
    },
    Tool {
        name: "cluster_health",
        description: "Check that the Zorvia API and its cluster connection are healthy.",
        method: "GET",
        path: "/health",
        body_keys: &[],
        read_only: true,
        destructive: false,
        idempotent: true,
        schema: schema_none,
    },
    Tool {
        name: "start_vm",
        description: "Power on a virtual machine.",
        method: "POST",
        path: "/vms/{name}/start",
        body_keys: &[],
        read_only: false,
        destructive: false,
        idempotent: true,
        schema: schema_vm,
    },
    Tool {
        name: "stop_vm",
        description: "Power off a virtual machine. Running workloads in the guest are interrupted.",
        method: "POST",
        path: "/vms/{name}/stop",
        body_keys: &[],
        read_only: false,
        destructive: false,
        idempotent: true,
        schema: schema_vm,
    },
    Tool {
        name: "restart_vm",
        description: "Restart a virtual machine. Running workloads in the guest are interrupted.",
        method: "POST",
        path: "/vms/{name}/restart",
        body_keys: &[],
        read_only: false,
        destructive: false,
        idempotent: false,
        schema: schema_vm,
    },
    Tool {
        name: "create_vm",
        description: "Create a virtual machine from an image with the given CPU, memory and disk.",
        method: "POST",
        path: "/vms",
        body_keys: &[
            "name",
            "image",
            "cpus",
            "memory",
            "disk",
            "start",
            "guest_os",
            "tenant",
            "labels",
            "expose_ssh",
        ],
        read_only: false,
        destructive: false,
        idempotent: false,
        schema: schema_create_vm,
    },
    Tool {
        name: "delete_vm",
        description: "Permanently delete a virtual machine. Requires confirm=true.",
        method: "DELETE",
        path: "/vms/{name}",
        body_keys: &[],
        read_only: false,
        destructive: true,
        idempotent: true,
        schema: schema_vm_confirm,
    },
    Tool {
        name: "create_snapshot",
        description: "Take a snapshot of a virtual machine.",
        method: "POST",
        path: "/vms/{name}/snapshots",
        body_keys: &["name", "description", "snapshot_type"],
        read_only: false,
        destructive: false,
        idempotent: false,
        schema: schema_create_snapshot,
    },
    Tool {
        name: "delete_snapshot",
        description: "Permanently delete a snapshot (a recovery point). Requires confirm=true.",
        method: "DELETE",
        path: "/vms/{name}/snapshots/{id}",
        body_keys: &[],
        read_only: false,
        destructive: true,
        idempotent: true,
        schema: schema_snapshot_confirm,
    },
    Tool {
        name: "revert_snapshot",
        description: "Revert a virtual machine to a snapshot, discarding changes made since. Requires confirm=true.",
        method: "POST",
        path: "/vms/{name}/snapshots/{id}/revert",
        body_keys: &[],
        read_only: false,
        destructive: true,
        idempotent: false,
        schema: schema_snapshot_confirm,
    },
];

fn tool_json(t: &Tool) -> Value {
    json!({
        "name": t.name,
        "description": t.description,
        "inputSchema": (t.schema)(),
        "annotations": {
            "readOnlyHint": t.read_only,
            "destructiveHint": t.destructive,
            "idempotentHint": t.idempotent,
            "openWorldHint": false
        }
    })
}

/// A path segment is a Kubernetes-style name. Anything else (slashes, dots-only, `..`, `?`, `#`,
/// `%`) is refused so an argument can never steer the request to a different REST route.
pub fn valid_segment(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 253
        && s != "."
        && s != ".."
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_'))
}

/// Build the REST path and body for a tool call, or explain what is wrong with the arguments.
pub fn build_request(
    tool: &Tool,
    args: &Map<String, Value>,
) -> Result<(String, Option<Value>), String> {
    let mut path = tool.path.to_string();
    for key in ["name", "id"] {
        let placeholder = format!("{{{key}}}");
        if !path.contains(&placeholder) {
            continue;
        }
        let value = args
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("missing required string argument '{key}'"))?;
        if !valid_segment(value) {
            return Err(format!(
                "argument '{key}' must be a plain name (letters, digits, '-', '.', '_')"
            ));
        }
        path = path.replace(&placeholder, value);
    }

    let body = if tool.body_keys.is_empty() {
        None
    } else {
        let mut map = Map::new();
        for key in tool.body_keys {
            // create_snapshot addresses the VM as `name`, so the snapshot's own name travels as
            // `snapshot_name` and is sent to the REST API as `name`.
            let (arg_key, body_key) = if tool.name == "create_snapshot" && *key == "name" {
                ("snapshot_name", "name")
            } else {
                (*key, *key)
            };
            if let Some(v) = args.get(arg_key).filter(|v| !v.is_null()) {
                map.insert(body_key.to_string(), v.clone());
            }
        }
        Some(Value::Object(map))
    };
    Ok((path, body))
}

fn rpc_result(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn rpc_error(id: &Value, code: i64, message: impl Into<String>) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message.into() } })
}

fn tool_text(text: String, is_error: bool) -> Value {
    let text = if text.chars().count() > MAX_RESULT_CHARS {
        let cut: String = text.chars().take(MAX_RESULT_CHARS).collect();
        format!("{cut}\n… [truncated]")
    } else {
        text
    };
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

/// Pull a human message out of a REST error body, whatever envelope it uses.
fn error_message(body: &Value) -> String {
    let candidates = [
        body.pointer("/error/message"),
        body.pointer("/message"),
        body.pointer("/error"),
        body.pointer("/data/message"),
    ];
    for c in candidates.into_iter().flatten() {
        if let Some(s) = c.as_str() {
            if !s.is_empty() {
                return s.to_string();
            }
        }
    }
    if body.is_null() {
        String::new()
    } else {
        body.to_string()
    }
}

async fn call_tool(
    params: &Value,
    disp: &dyn Dispatcher,
    cfg: &McpConfig,
) -> Result<Value, (i64, String)> {
    let name = params.get("name").and_then(Value::as_str).ok_or((
        INVALID_PARAMS,
        "tools/call requires a string 'name'".to_string(),
    ))?;
    let tool = TOOLS
        .iter()
        .find(|t| t.name == name && !cfg.disabled_tools.contains(t.name))
        .ok_or((INVALID_PARAMS, format!("Unknown tool: {name}")))?;

    let empty = Map::new();
    let args = match params.get("arguments") {
        None | Some(Value::Null) => &empty,
        Some(Value::Object(m)) => m,
        Some(_) => return Err((INVALID_PARAMS, "'arguments' must be an object".to_string())),
    };

    if tool.destructive && args.get("confirm").and_then(Value::as_bool) != Some(true) {
        return Ok(tool_text(
            format!(
                "Refused: {} is destructive and was not confirmed. Explain to the user what it will do, \
                 and call it again with confirm=true only after they agree.",
                tool.name
            ),
            true,
        ));
    }

    let (path, body) = match build_request(tool, args) {
        Ok(r) => r,
        Err(msg) => return Ok(tool_text(format!("Invalid arguments: {msg}"), true)),
    };

    let res = disp.dispatch(tool.method, path, body).await;
    if (200..300).contains(&res.status) {
        let text = if res.body.is_null() {
            format!("OK ({})", res.status)
        } else {
            serde_json::to_string_pretty(&res.body).unwrap_or_else(|_| res.body.to_string())
        };
        Ok(tool_text(text, false))
    } else {
        let msg = error_message(&res.body);
        let hint = match res.status {
            401 => " (the credential was rejected)",
            403 => " (the credential lacks permission for this action)",
            404 => " (not found)",
            _ => "",
        };
        Ok(tool_text(format!("HTTP {}{hint}: {msg}", res.status), true))
    }
}

async fn handle_one(msg: &Value, disp: &dyn Dispatcher, cfg: &McpConfig) -> Option<Value> {
    let Some(obj) = msg.as_object() else {
        return Some(rpc_error(
            &Value::Null,
            INVALID_REQUEST,
            "request must be a JSON object",
        ));
    };
    // A message with no id is a notification: it never gets a response.
    let id = obj.get("id").cloned();
    let method = obj.get("method").and_then(Value::as_str);
    let Some(method) = method else {
        // A response from the client to a server request. We send none, so ignore it.
        return id.map(|id| rpc_error(&id, INVALID_REQUEST, "missing 'method'"));
    };
    let params = obj.get("params").cloned().unwrap_or(Value::Null);

    let result: Result<Value, (i64, String)> = match method {
        "initialize" => {
            let requested = params.get("protocolVersion").and_then(Value::as_str);
            let version = requested
                .filter(|v| SUPPORTED_VERSIONS.contains(v))
                .unwrap_or(LATEST_PROTOCOL_VERSION);
            Ok(json!({
                "protocolVersion": version,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "zorvia", "version": env!("CARGO_PKG_VERSION") },
                "instructions": INSTRUCTIONS
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => {
            let tools: Vec<Value> = TOOLS
                .iter()
                .filter(|t| !cfg.disabled_tools.contains(t.name))
                .map(tool_json)
                .collect();
            Ok(json!({ "tools": tools }))
        }
        "tools/call" => call_tool(&params, disp, cfg).await,
        m if m.starts_with("notifications/") => return None,
        other => Err((METHOD_NOT_FOUND, format!("Method not found: {other}"))),
    };

    let id = id?;
    Some(match result {
        Ok(r) => rpc_result(&id, r),
        Err((code, msg)) => rpc_error(&id, code, msg),
    })
}

/// Handle one JSON-RPC payload (a single message or a batch). Returns `None` when nothing must be
/// sent back (notifications only).
pub async fn handle_payload(
    payload: &Value,
    disp: &dyn Dispatcher,
    cfg: &McpConfig,
) -> Option<Value> {
    match payload {
        Value::Array(items) if items.is_empty() => {
            Some(rpc_error(&Value::Null, INVALID_REQUEST, "empty batch"))
        }
        Value::Array(items) => {
            let mut out = Vec::new();
            for item in items {
                if let Some(r) = handle_one(item, disp, cfg).await {
                    out.push(r);
                }
            }
            if out.is_empty() {
                None
            } else {
                Some(Value::Array(out))
            }
        }
        other => handle_one(other, disp, cfg).await,
    }
}

/// Response for a body that is not valid JSON.
pub fn parse_error() -> Value {
    rpc_error(&Value::Null, PARSE_ERROR, "Parse error")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Records every REST call and answers with a canned result.
    struct Canned {
        calls: Mutex<Vec<(String, String, Option<Value>)>>,
        reply: DispatchResult,
    }

    impl Canned {
        fn new(status: u16, body: Value) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                reply: DispatchResult { status, body },
            }
        }
        fn calls(&self) -> Vec<(String, String, Option<Value>)> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl Dispatcher for Canned {
        fn dispatch<'a>(
            &'a self,
            method: &'static str,
            path: String,
            body: Option<Value>,
        ) -> BoxFuture<'a, DispatchResult> {
            self.calls
                .lock()
                .unwrap()
                .push((method.to_string(), path, body));
            let r = self.reply.clone();
            Box::pin(async move { r })
        }
    }

    fn call(name: &str, args: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": { "name": name, "arguments": args } })
    }

    async fn run(msg: Value, d: &Canned) -> Option<Value> {
        handle_payload(&msg, d, &McpConfig::default()).await
    }

    #[tokio::test]
    async fn initialize_negotiates_version_and_advertises_tools() {
        let d = Canned::new(200, Value::Null);
        let r = run(
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}),
            &d,
        )
        .await
        .unwrap();
        assert_eq!(r["result"]["protocolVersion"], "2024-11-05");
        assert!(r["result"]["capabilities"]["tools"].is_object());
        assert_eq!(r["result"]["serverInfo"]["name"], "zorvia");

        let r = run(
            json!({"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}),
            &d,
        )
        .await
        .unwrap();
        assert_eq!(r["result"]["protocolVersion"], LATEST_PROTOCOL_VERSION);
    }

    #[tokio::test]
    async fn notifications_get_no_response() {
        let d = Canned::new(200, Value::Null);
        assert!(run(
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            &d
        )
        .await
        .is_none());
    }

    #[tokio::test]
    async fn tools_list_has_valid_schemas_and_annotations() {
        let d = Canned::new(200, Value::Null);
        let r = run(json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}), &d)
            .await
            .unwrap();
        let tools = r["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), TOOLS.len());
        for t in tools {
            assert_eq!(t["inputSchema"]["type"], "object", "{}", t["name"]);
            assert!(t["annotations"]["readOnlyHint"].is_boolean());
        }
        let del = tools.iter().find(|t| t["name"] == "delete_vm").unwrap();
        assert_eq!(del["annotations"]["destructiveHint"], true);
        assert_eq!(del["inputSchema"]["required"], json!(["name", "confirm"]));
    }

    #[test]
    fn tool_names_are_unique_and_destructive_tools_require_confirm() {
        let mut seen = HashSet::new();
        for t in TOOLS {
            assert!(seen.insert(t.name), "duplicate tool {}", t.name);
            assert!(!(t.read_only && t.destructive), "{}", t.name);
            if t.destructive {
                let schema = (t.schema)();
                assert!(
                    schema["required"]
                        .as_array()
                        .unwrap()
                        .contains(&json!("confirm")),
                    "{} must require confirm",
                    t.name
                );
            }
            if t.read_only {
                assert_eq!(t.method, "GET", "{}", t.name);
            }
        }
    }

    #[tokio::test]
    async fn read_tool_maps_to_get_and_returns_body() {
        let d = Canned::new(200, json!({"name":"web-1","status":"Running"}));
        let r = run(call("get_vm", json!({"name":"web-1"})), &d)
            .await
            .unwrap();
        assert_eq!(d.calls(), vec![("GET".into(), "/vms/web-1".into(), None)]);
        assert_eq!(r["result"]["isError"], false);
        assert!(r["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Running"));
    }

    #[tokio::test]
    async fn power_tool_posts_without_body() {
        let d = Canned::new(204, Value::Null);
        let r = run(call("start_vm", json!({"name":"web-1"})), &d)
            .await
            .unwrap();
        assert_eq!(
            d.calls(),
            vec![("POST".into(), "/vms/web-1/start".into(), None)]
        );
        assert_eq!(r["result"]["content"][0]["text"], "OK (204)");
    }

    #[tokio::test]
    async fn create_vm_forwards_only_declared_keys() {
        let d = Canned::new(201, json!({"name":"x"}));
        run(
            call(
                "create_vm",
                json!({"name":"x","image":"quay.io/a:1","cpus":2,"memory":2048,"cpu_dedicated_placement":true,"unknown":1}),
            ),
            &d,
        )
        .await
        .unwrap();
        let calls = d.calls();
        assert_eq!(calls[0].0, "POST");
        assert_eq!(calls[0].1, "/vms");
        let body = calls[0].2.clone().unwrap();
        assert_eq!(body["cpus"], 2);
        assert!(body.get("cpu_dedicated_placement").is_none());
        assert!(body.get("unknown").is_none());
    }

    #[tokio::test]
    async fn create_snapshot_sends_snapshot_name_as_name() {
        let d = Canned::new(201, json!({}));
        run(
            call(
                "create_snapshot",
                json!({"name":"web-1","snapshot_name":"before-upgrade","snapshot_type":"Disk"}),
            ),
            &d,
        )
        .await
        .unwrap();
        let calls = d.calls();
        assert_eq!(calls[0].1, "/vms/web-1/snapshots");
        let body = calls[0].2.clone().unwrap();
        assert_eq!(body["name"], "before-upgrade");
        assert_eq!(body["snapshot_type"], "Disk");
    }

    #[tokio::test]
    async fn destructive_tools_refuse_without_confirm_and_never_call_rest() {
        let d = Canned::new(204, Value::Null);
        for (tool, args) in [
            ("delete_vm", json!({"name":"web-1"})),
            ("delete_vm", json!({"name":"web-1","confirm":false})),
            ("delete_vm", json!({"name":"web-1","confirm":"true"})),
            ("delete_snapshot", json!({"name":"web-1","id":"s1"})),
            ("revert_snapshot", json!({"name":"web-1","id":"s1"})),
        ] {
            let r = run(call(tool, args), &d).await.unwrap();
            assert_eq!(r["result"]["isError"], true, "{tool}");
            assert!(r["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("confirm=true"));
        }
        assert!(d.calls().is_empty());

        run(
            call("delete_vm", json!({"name":"web-1","confirm":true})),
            &d,
        )
        .await
        .unwrap();
        assert_eq!(
            d.calls(),
            vec![("DELETE".into(), "/vms/web-1".into(), None)]
        );
    }

    #[tokio::test]
    async fn path_arguments_cannot_escape_their_segment() {
        let d = Canned::new(200, Value::Null);
        for bad in [
            "../v1/users",
            "a/b",
            "..",
            ".",
            "",
            "a?x=1",
            "a#b",
            "a%2fb",
            "a b",
        ] {
            let r = run(call("get_vm", json!({"name": bad})), &d).await.unwrap();
            assert_eq!(r["result"]["isError"], true, "{bad:?}");
        }
        let r = run(call("get_vm", json!({})), &d).await.unwrap();
        assert_eq!(r["result"]["isError"], true);
        assert!(d.calls().is_empty());
    }

    #[tokio::test]
    async fn rest_errors_surface_as_tool_errors_with_a_hint() {
        let d = Canned::new(
            403,
            json!({"error":"FORBIDDEN","message":"Missing permission: vm.power"}),
        );
        let r = run(call("stop_vm", json!({"name":"web-1"})), &d)
            .await
            .unwrap();
        assert_eq!(r["result"]["isError"], true);
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            text.contains("403") && text.contains("Missing permission: vm.power"),
            "{text}"
        );
    }

    #[tokio::test]
    async fn unknown_and_disabled_tools_are_invalid_params() {
        let d = Canned::new(200, Value::Null);
        let r = run(call("drop_database", json!({})), &d).await.unwrap();
        assert_eq!(r["error"]["code"], INVALID_PARAMS);

        let cfg = McpConfig {
            disabled_tools: ["delete_vm".to_string()].into_iter().collect(),
        };
        let r = handle_payload(
            &call("delete_vm", json!({"name":"a","confirm":true})),
            &d,
            &cfg,
        )
        .await
        .unwrap();
        assert_eq!(r["error"]["code"], INVALID_PARAMS);
        let list = handle_payload(
            &json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
            &d,
            &cfg,
        )
        .await
        .unwrap();
        assert!(!list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "delete_vm"));
        assert!(d.calls().is_empty());
    }

    #[tokio::test]
    async fn unknown_method_and_batches() {
        let d = Canned::new(200, Value::Null);
        let r = run(
            json!({"jsonrpc":"2.0","id":1,"method":"resources/list"}),
            &d,
        )
        .await
        .unwrap();
        assert_eq!(r["error"]["code"], METHOD_NOT_FOUND);

        let r = run(
            json!([
                {"jsonrpc":"2.0","id":1,"method":"ping"},
                {"jsonrpc":"2.0","method":"notifications/initialized"}
            ]),
            &d,
        )
        .await
        .unwrap();
        assert_eq!(r.as_array().unwrap().len(), 1);
        assert!(run(json!([]), &d).await.unwrap()["error"]["code"] == INVALID_REQUEST);
    }

    #[test]
    fn long_results_are_truncated() {
        let v = tool_text("x".repeat(MAX_RESULT_CHARS + 50), false);
        assert!(v["content"][0]["text"]
            .as_str()
            .unwrap()
            .ends_with("[truncated]"));
    }
}
