//! `zorvia mcp`: stdio bridge for MCP clients that launch a local command (Claude Desktop, Claude
//! Code, IDE plugins). It forwards each JSON-RPC line from stdin to `POST <url>/api/mcp` with the
//! token as `x-api-key` and writes the reply to stdout. All authorization happens on the server, so
//! the bridge holds no permissions of its own. Logs go to stderr; stdout carries protocol only.

use anyhow::Result;

#[cfg(feature = "web")]
pub async fn run_stdio_bridge(
    url: String,
    token: String,
    insecure: bool,
    ca_cert: Option<String>,
) -> Result<()> {
    use serde_json::{json, Value};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let endpoint = format!("{}/api/mcp", url.trim_end_matches('/'));
    let mut builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .danger_accept_invalid_certs(insecure);
    if let Some(path) = ca_cert {
        let pem = std::fs::read(&path)
            .map_err(|e| anyhow::anyhow!("cannot read --ca-cert {path}: {e}"))?;
        builder = builder.add_root_certificate(reqwest::Certificate::from_pem(&pem)?);
    }
    let client = builder.build()?;
    eprintln!("zorvia mcp: bridging stdio to {endpoint}");

    let rpc_error = |id: Value, code: i64, message: String| -> String {
        json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
            .to_string()
    };

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();
    while let Some(line) = lines.next_line().await? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => {
                let out = rpc_error(Value::Null, -32700, "Parse error".to_string());
                stdout.write_all(out.as_bytes()).await?;
                stdout.write_all(b"\n").await?;
                stdout.flush().await?;
                continue;
            }
        };
        // Only requests (which carry an id) are answered; notifications never get a reply.
        let id = msg.get("id").cloned();

        let reply = client
            .post(&endpoint)
            .header("x-api-key", &token)
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .body(line.to_string())
            .send()
            .await;

        let out: Option<String> = match reply {
            Ok(r) => {
                let status = r.status();
                let text = r.text().await.unwrap_or_default();
                if status == reqwest::StatusCode::ACCEPTED
                    || status == reqwest::StatusCode::NO_CONTENT
                {
                    None
                } else if status.is_success() {
                    // Re-serialize compactly: one JSON message per line on stdout.
                    serde_json::from_str::<Value>(&text)
                        .ok()
                        .map(|v| v.to_string())
                } else {
                    let hint = match status.as_u16() {
                        401 => " (check ZORVIA_TOKEN)",
                        404 => " (MCP is not enabled on the server: set ZORVIA_MCP_ENABLED=1)",
                        _ => "",
                    };
                    id.map(|id| rpc_error(id, -32000, format!("HTTP {status}{hint}")))
                }
            }
            Err(e) => id.map(|id| {
                rpc_error(
                    id,
                    -32000,
                    format!("cannot reach Zorvia at {endpoint}: {e}"),
                )
            }),
        };
        if let Some(out) = out {
            stdout.write_all(out.as_bytes()).await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
        }
    }
    Ok(())
}

#[cfg(not(feature = "web"))]
pub async fn run_stdio_bridge(
    _url: String,
    _token: String,
    _insecure: bool,
    _ca_cert: Option<String>,
) -> Result<()> {
    anyhow::bail!("this build has no HTTP client; rebuild with the default `web` feature")
}
