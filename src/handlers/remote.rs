// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! CLI for the experimental features that are reached through the REST API: DR pairs, volume
//! encryption and rescue operations (`zorvia dr|volume|rescue ...`). Each command builds one
//! request against a running Zorvia API (`--url`/`ZORVIA_URL`, `--token`/`ZORVIA_TOKEN`), so the
//! server stays the only place that enforces RBAC, validation and the experimental gate.
//!
//! Secrets: the rescue password is never an argument or an environment variable. It is read from
//! a no-echo prompt (terminal) or one line of stdin (pipe), sent only in the request body, held in
//! a type whose `Debug` is redacted, and scrubbed from any error text before it is shown.

use crate::cli::{
    DrCommands, DrPairCommands, PoolCommands, RemoteArgs, RescueCommands, VolumeCommands,
    VolumeEncryptionCommands,
};
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Map, Value};

const DR_BASE: &str = "/api/v1/enterprise/dr";
const VE_BASE: &str = "/api/volume-encryption";
const POOL_BASE: &str = "/api/v1/inventory/pools";

/// A secret string whose `Debug`/`Display` never show the value.
pub struct Secret(String);

impl Secret {
    pub fn new(s: String) -> Self {
        Secret(s)
    }
    fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(***)")
    }
}

/// One API request, built without touching the network.
#[derive(Debug, Clone, PartialEq)]
pub struct Req {
    pub method: &'static str,
    pub path: String,
    pub body: Option<Value>,
}

fn get(path: String) -> Req {
    Req {
        method: "GET",
        path,
        body: None,
    }
}

fn delete(path: String) -> Req {
    Req {
        method: "DELETE",
        path,
        body: None,
    }
}

fn post(path: String, body: Value) -> Req {
    Req {
        method: "POST",
        path,
        body: Some(body),
    }
}

/// DNS-1123 style name; also guarantees a safe URL path segment.
fn check_name(what: &str, s: &str) -> Result<()> {
    let ok = !s.is_empty()
        && s.len() <= 253
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.')
        && !s.starts_with(['-', '.'])
        && !s.ends_with(['-', '.']);
    if !ok {
        bail!(
            "{what} '{s}' is not a valid Kubernetes name (lowercase letters, digits, '-' and '.')"
        );
    }
    Ok(())
}

// ---------------------------------------------------------------- request construction

pub fn dr_request(cmd: &DrPairCommands) -> Result<Req> {
    Ok(match cmd {
        DrPairCommands::List => get(format!("{DR_BASE}/pairs")),
        DrPairCommands::Create {
            name,
            vm,
            namespace,
            dr_namespace,
            rpo_seconds,
            rto_seconds,
        } => {
            check_name("pair name", name)?;
            check_name("vm", vm)?;
            if !(60..=604_800).contains(rpo_seconds) {
                bail!("--rpo-seconds must be between 60 and 604800");
            }
            let mut b = Map::new();
            b.insert("name".into(), json!(name));
            b.insert("vm".into(), json!(vm));
            b.insert("rpo_seconds".into(), json!(rpo_seconds));
            if let Some(ns) = namespace {
                check_name("namespace", ns)?;
                b.insert("namespace".into(), json!(ns));
            }
            if let Some(ns) = dr_namespace {
                check_name("dr namespace", ns)?;
                b.insert("dr_namespace".into(), json!(ns));
            }
            if let Some(r) = rto_seconds {
                b.insert("rto_seconds".into(), json!(r));
            }
            post(format!("{DR_BASE}/pairs"), Value::Object(b))
        }
        DrPairCommands::Status { name } => {
            check_name("pair name", name)?;
            get(format!("{DR_BASE}/pairs/{name}/status"))
        }
        DrPairCommands::Failover {
            name,
            execute,
            confirm,
            no_start,
        } => {
            check_name("pair name", name)?;
            let phrase = format!("failover {name}");
            let mut b = Map::new();
            if *execute {
                // A real failover is never implicit: the exact phrase must be typed.
                if confirm.as_deref() != Some(phrase.as_str()) {
                    bail!(
                        "--execute needs --confirm \"{phrase}\" (the exact phrase); \
                         run without --execute first to see the plan"
                    );
                }
                b.insert("dry_run".into(), json!(false));
                b.insert("confirm".into(), json!(phrase));
                b.insert("start".into(), json!(!no_start));
            } else {
                if confirm.is_some() || *no_start {
                    bail!("--confirm and --no-start only apply together with --execute");
                }
                b.insert("dry_run".into(), json!(true));
            }
            post(format!("{DR_BASE}/pairs/{name}/failover"), Value::Object(b))
        }
        DrPairCommands::FailbackPlan { name } => {
            check_name("pair name", name)?;
            get(format!("{DR_BASE}/pairs/{name}/failback/plan"))
        }
        DrPairCommands::Resume { name, confirm } => {
            check_name("pair name", name)?;
            let phrase = format!("resume {name}");
            if confirm != &phrase {
                bail!("--confirm must be exactly \"{phrase}\"");
            }
            post(
                format!("{DR_BASE}/pairs/{name}/resume"),
                json!({ "confirm": phrase }),
            )
        }
    })
}

pub fn pool_request(cmd: &PoolCommands) -> Result<Req> {
    let assign = |pool: &String, vm: &String, namespace: &Option<String>, remove: bool| {
        check_name("pool", pool)?;
        check_name("vm", vm)?;
        let mut b = Map::new();
        b.insert("vm".into(), json!(vm));
        if let Some(ns) = namespace {
            check_name("namespace", ns)?;
            b.insert("namespace".into(), json!(ns));
        }
        if remove {
            b.insert("remove".into(), json!(true));
        }
        Ok::<Req, anyhow::Error>(post(format!("{POOL_BASE}/{pool}/assign"), Value::Object(b)))
    };
    Ok(match cmd {
        PoolCommands::List => get(POOL_BASE.to_string()),
        PoolCommands::Create {
            name,
            description,
            max_vms,
            max_vcpus,
            max_memory_mib,
        } => {
            check_name("pool", name)?;
            let mut b = Map::new();
            b.insert("name".into(), json!(name));
            if let Some(d) = description {
                b.insert("description".into(), json!(d));
            }
            for (key, v) in [
                ("max_vms", max_vms),
                ("max_vcpus", max_vcpus),
                ("max_memory_mib", max_memory_mib),
            ] {
                if let Some(v) = v {
                    if *v == 0 {
                        bail!("--{} must be at least 1", key.replace('_', "-"));
                    }
                    b.insert(key.into(), json!(v));
                }
            }
            post(POOL_BASE.to_string(), Value::Object(b))
        }
        PoolCommands::Show { name } => {
            check_name("pool", name)?;
            get(format!("{POOL_BASE}/{name}"))
        }
        PoolCommands::Report { name } => {
            check_name("pool", name)?;
            get(format!("{POOL_BASE}/{name}/report"))
        }
        PoolCommands::Hierarchy => get(format!("{POOL_BASE}/hierarchy")),
        PoolCommands::Assign {
            pool,
            vm,
            namespace,
        } => assign(pool, vm, namespace, false)?,
        PoolCommands::Unassign {
            pool,
            vm,
            namespace,
        } => assign(pool, vm, namespace, true)?,
        PoolCommands::Delete { name, unassign } => {
            check_name("pool", name)?;
            let q = if *unassign { "?unassign=true" } else { "" };
            delete(format!("{POOL_BASE}/{name}{q}"))
        }
    })
}

pub fn volume_request(cmd: &VolumeEncryptionCommands, namespace: &str) -> Result<Req> {
    check_name("namespace", namespace)?;
    let ident = |vm: &str, volume: &str| -> Result<()> {
        check_name("vm", vm)?;
        check_name("volume", volume)
    };
    Ok(match cmd {
        VolumeEncryptionCommands::Status => get(format!("{VE_BASE}/status")),
        VolumeEncryptionCommands::Plan {
            vm,
            volume,
            size,
            storage_class,
            acknowledge_unverified_encryption,
        }
        | VolumeEncryptionCommands::Apply {
            vm,
            volume,
            size,
            storage_class,
            acknowledge_unverified_encryption,
            ..
        } => {
            ident(vm, volume)?;
            check_name("storage class", storage_class)?;
            let is_apply = matches!(cmd, VolumeEncryptionCommands::Apply { .. });
            if let VolumeEncryptionCommands::Apply { yes: false, .. } = cmd {
                bail!(
                    "apply creates a key Secret and a DataVolume; run `plan` first, then add --yes"
                );
            }
            let body = json!({
                "namespace": namespace,
                "vm": vm,
                "volume": volume,
                "size": size,
                "storage_class": storage_class,
                "mode": "storage_class",
                "acknowledge_unverified_encryption": acknowledge_unverified_encryption,
            });
            let verb = if is_apply { "apply" } else { "plan" };
            post(format!("{VE_BASE}/{verb}"), body)
        }
        VolumeEncryptionCommands::Rotate { vm, volume, yes } => {
            ident(vm, volume)?;
            if !yes {
                bail!("rotate re-wraps the volume's key Secret; add --yes to confirm");
            }
            post(
                format!("{VE_BASE}/rotate"),
                json!({ "namespace": namespace, "vm": vm, "volume": volume }),
            )
        }
    })
}

pub fn rescue_request(cmd: &RescueCommands, password: Option<&Secret>) -> Result<Req> {
    use crate::kube::rescue::logic;
    Ok(match cmd {
        RescueCommands::ResetPassword { vm, user } => {
            check_name("vm", vm)?;
            logic::validate_username(user).map_err(|e| anyhow!("{e}"))?;
            let pw = password.ok_or_else(|| anyhow!("no password supplied"))?;
            logic::validate_password(pw.expose()).map_err(|e| anyhow!("{e}"))?;
            post(
                format!("/api/vms/{vm}/rescue"),
                json!({ "operation": "reset-password", "user": user.trim(), "password": pw.expose() }),
            )
        }
        RescueCommands::InstallPackages { vm, packages } => {
            check_name("vm", vm)?;
            let pkgs = logic::validate_packages(packages).map_err(|e| anyhow!("{e}"))?;
            post(
                format!("/api/vms/{vm}/rescue"),
                json!({ "operation": "install-packages", "packages": pkgs }),
            )
        }
        RescueCommands::Inspect { vm } => {
            check_name("vm", vm)?;
            post(
                format!("/api/vms/{vm}/rescue"),
                json!({ "operation": "inspect" }),
            )
        }
        RescueCommands::Status { vm, job } => {
            check_name("vm", vm)?;
            check_name("job", job)?;
            get(format!("/api/vms/{vm}/rescue/{job}"))
        }
    })
}

// ---------------------------------------------------------------- password input

/// Reads one password line from `r` (strips only the trailing line ending).
pub fn read_password_line<R: std::io::BufRead>(r: &mut R) -> Result<Secret> {
    let mut line = String::new();
    if r.read_line(&mut line)? == 0 {
        bail!("no password on stdin");
    }
    while line.ends_with('\n') || line.ends_with('\r') {
        line.pop();
    }
    Ok(Secret::new(line))
}

#[cfg(feature = "web")]
fn obtain_password(user: &str) -> Result<Secret> {
    use std::io::IsTerminal;
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        let a = rpassword::prompt_password(format!("New password for '{user}': "))?;
        let b = rpassword::prompt_password("Repeat password: ")?;
        if a != b {
            bail!("passwords do not match");
        }
        Ok(Secret::new(a))
    } else {
        read_password_line(&mut stdin.lock())
    }
}

// ---------------------------------------------------------------- HTTP

#[cfg(feature = "web")]
pub struct Remote {
    base: String,
    token: String,
    client: reqwest::Client,
}

#[cfg(feature = "web")]
impl Remote {
    pub fn new(args: &RemoteArgs) -> Result<Self> {
        let url = args
            .url
            .clone()
            .ok_or_else(|| anyhow!("no API URL: pass --url or set ZORVIA_URL"))?;
        let token = args
            .token
            .clone()
            .ok_or_else(|| anyhow!("no API token: set ZORVIA_TOKEN (or pass --token)"))?;
        let mut b = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::none())
            .danger_accept_invalid_certs(args.insecure);
        if let Some(path) = &args.ca_cert {
            let pem =
                std::fs::read(path).map_err(|e| anyhow!("cannot read --ca-cert {path}: {e}"))?;
            b = b.add_root_certificate(reqwest::Certificate::from_pem(&pem)?);
        }
        Ok(Self {
            base: url.trim_end_matches('/').to_string(),
            token,
            client: b.build()?,
        })
    }

    /// Sends `req`; returns the response `data` (or the whole body). `redact` strings are removed
    /// from every error message.
    pub async fn send(&self, req: &Req, redact: &[&str]) -> Result<Value> {
        let url = format!("{}{}", self.base, req.path);
        let mut rb = match req.method {
            "GET" => self.client.get(&url),
            "DELETE" => self.client.delete(&url),
            _ => self.client.post(&url),
        }
        .bearer_auth(&self.token);
        if let Some(b) = &req.body {
            rb = rb.json(b);
        }
        let scrub = |s: String| -> String {
            let mut s = s;
            for r in redact.iter().filter(|r| !r.is_empty()) {
                s = s.replace(r, "***");
            }
            s
        };
        let resp = rb.send().await.map_err(|e| {
            anyhow!(
                "cannot reach Zorvia at {}: {}",
                self.base,
                scrub(e.without_url().to_string())
            )
        })?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let parsed: Option<Value> = serde_json::from_str(&text).ok();
        if status.is_success() {
            if text.trim().is_empty() {
                return Ok(json!({ "ok": true, "status": status.as_u16() }));
            }
            let v =
                parsed.ok_or_else(|| anyhow!("server answered {status} with a non-JSON body"))?;
            return Ok(match v.get("data") {
                Some(d) => d.clone(),
                None => v,
            });
        }
        let (code, message) = parsed
            .as_ref()
            .and_then(|v| v.get("error"))
            .map(|e| {
                (
                    e.get("code")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    e.get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                )
            })
            .unwrap_or_default();
        let code = scrub(code);
        let message = scrub(message);
        if status.as_u16() == 403 && code == "EXPERIMENTAL_DISABLED" {
            bail!(
                "refused by the server: this feature is Experimental and disabled ({message}). \
                 An operator must set ZORVIA_EXPERIMENTAL=1 on the Zorvia API server."
            );
        }
        let hint = match status.as_u16() {
            401 => " (check ZORVIA_TOKEN)",
            403 => " (the token needs cluster.admin)",
            _ => "",
        };
        let detail = match (code.is_empty(), message.is_empty()) {
            (true, true) => String::new(),
            (false, true) => format!(" {code}"),
            (true, false) => format!(" {message}"),
            (false, false) => format!(" {code}: {message}"),
        };
        bail!("HTTP {status}{detail}{hint}")
    }
}

// ---------------------------------------------------------------- output

/// `json`, `yaml`, or a readable text rendering (`table`).
pub fn render(v: &Value, output: &str) -> Result<String> {
    match output {
        "json" => Ok(serde_json::to_string_pretty(v)?),
        "yaml" => Ok(serde_yaml::to_string(v)?),
        "table" | "text" => {
            let mut out = String::new();
            text(v, 0, &mut out);
            Ok(out.trim_end().to_string())
        }
        other => bail!("unknown output format '{other}'; expected table, yaml or json"),
    }
}

fn scalar(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "-".into(),
        other => other.to_string(),
    }
}

fn text(v: &Value, indent: usize, out: &mut String) {
    let pad = "  ".repeat(indent);
    match v {
        Value::Object(m) => {
            for (k, val) in m {
                match val {
                    Value::Object(_) | Value::Array(_) if !is_empty(val) => {
                        out.push_str(&format!("{pad}{k}:\n"));
                        text(val, indent + 1, out);
                    }
                    _ => out.push_str(&format!("{pad}{k}: {}\n", scalar(val))),
                }
            }
        }
        Value::Array(a) => {
            for item in a {
                match item {
                    Value::Object(_) | Value::Array(_) => {
                        out.push_str(&format!("{pad}-\n"));
                        text(item, indent + 1, out);
                    }
                    _ => out.push_str(&format!("{pad}- {}\n", scalar(item))),
                }
            }
        }
        other => out.push_str(&format!("{pad}{}\n", scalar(other))),
    }
}

fn is_empty(v: &Value) -> bool {
    match v {
        Value::Object(m) => m.is_empty(),
        Value::Array(a) => a.is_empty(),
        _ => false,
    }
}

// ---------------------------------------------------------------- entry points

#[cfg(feature = "web")]
async fn finish(remote: &RemoteArgs, req: Req, redact: &[&str]) -> Result<()> {
    let r = Remote::new(remote)?;
    let v = r.send(&req, redact).await?;
    println!("{}", render(&v, &remote.output)?);
    Ok(())
}

#[cfg(feature = "web")]
pub async fn run_dr(remote: RemoteArgs, cmd: DrCommands) -> Result<()> {
    let DrCommands::Pair { cmd } = cmd;
    finish(&remote, dr_request(&cmd)?, &[]).await
}

#[cfg(feature = "web")]
pub async fn run_pool(remote: RemoteArgs, cmd: PoolCommands) -> Result<()> {
    finish(&remote, pool_request(&cmd)?, &[]).await
}

#[cfg(feature = "web")]
pub async fn run_volume(remote: RemoteArgs, cmd: VolumeCommands, namespace: &str) -> Result<()> {
    let VolumeCommands::Encryption { cmd } = cmd;
    finish(&remote, volume_request(&cmd, namespace)?, &[]).await
}

#[cfg(feature = "web")]
pub async fn run_rescue(remote: RemoteArgs, cmd: RescueCommands) -> Result<()> {
    let password = match &cmd {
        RescueCommands::ResetPassword { user, .. } => Some(obtain_password(user)?),
        _ => None,
    };
    let req = rescue_request(&cmd, password.as_ref())?;
    let redact: Vec<&str> = password.iter().map(Secret::expose).collect();
    finish(&remote, req, &redact).await
}

#[cfg(not(feature = "web"))]
pub async fn run_dr(_: RemoteArgs, _: DrCommands) -> Result<()> {
    bail!("this build has no HTTP client; rebuild with the default `web` feature")
}
#[cfg(not(feature = "web"))]
pub async fn run_pool(_: RemoteArgs, _: PoolCommands) -> Result<()> {
    bail!("this build has no HTTP client; rebuild with the default `web` feature")
}
#[cfg(not(feature = "web"))]
pub async fn run_volume(_: RemoteArgs, _: VolumeCommands, _: &str) -> Result<()> {
    bail!("this build has no HTTP client; rebuild with the default `web` feature")
}
#[cfg(not(feature = "web"))]
pub async fn run_rescue(_: RemoteArgs, _: RescueCommands) -> Result<()> {
    bail!("this build has no HTTP client; rebuild with the default `web` feature")
}

#[cfg(all(test, feature = "web"))]
mod tests {
    use super::*;
    use crate::cli::{Cli, Commands};
    use clap::Parser;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Debug)]
    struct Seen {
        method: String,
        path: String,
        auth: String,
        body: String,
    }

    /// Mock API: answers every request with `(status, body)` and records what it saw.
    async fn mock(status: u16, body: Value) -> (String, Arc<Mutex<Vec<Seen>>>) {
        use axum::{
            extract::State, http::HeaderMap, http::Method, http::Uri, response::IntoResponse,
        };
        type St = (Arc<Mutex<Vec<Seen>>>, u16, Value);
        async fn h(
            State(st): State<St>,
            m: Method,
            u: Uri,
            hd: HeaderMap,
            b: String,
        ) -> impl IntoResponse {
            st.0.lock().unwrap().push(Seen {
                method: m.to_string(),
                path: u.path().to_string(),
                auth: hd
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string(),
                body: b,
            });
            (
                axum::http::StatusCode::from_u16(st.1).unwrap(),
                axum::Json(st.2.clone()),
            )
        }
        let seen = Arc::new(Mutex::new(Vec::new()));
        let app = axum::Router::new()
            .fallback(h)
            .with_state((seen.clone(), status, body));
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(l, app).await.unwrap();
        });
        (format!("http://{addr}"), seen)
    }

    fn remote(url: &str) -> Remote {
        Remote::new(&RemoteArgs {
            url: Some(url.into()),
            token: Some("tok-123".into()),
            insecure: false,
            ca_cert: None,
            output: "json".into(),
        })
        .unwrap()
    }

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        let mut v = vec!["zorvia"];
        v.extend_from_slice(args);
        Cli::try_parse_from(v)
    }

    fn dr_cmd(args: &[&str]) -> DrPairCommands {
        match *parse(args).unwrap().command {
            Commands::Dr {
                cmd: DrCommands::Pair { cmd },
                ..
            } => cmd,
            _ => panic!("not a dr command"),
        }
    }

    fn ve_cmd(args: &[&str]) -> VolumeEncryptionCommands {
        match *parse(args).unwrap().command {
            Commands::Volume {
                cmd: VolumeCommands::Encryption { cmd },
                ..
            } => cmd,
            _ => panic!("not a volume command"),
        }
    }

    fn rescue_cmd(args: &[&str]) -> RescueCommands {
        match *parse(args).unwrap().command {
            Commands::Rescue { cmd, .. } => cmd,
            _ => panic!("not a rescue command"),
        }
    }

    #[tokio::test]
    async fn dr_create_builds_request_with_bearer_token() {
        let (url, seen) = mock(200, json!({"success": true, "data": {"name": "p1"}})).await;
        let cmd = dr_cmd(&[
            "dr",
            "pair",
            "create",
            "p1",
            "--vm",
            "web",
            "--rpo-seconds",
            "900",
            "--dr-namespace",
            "dr-ns",
        ]);
        let v = remote(&url)
            .send(&dr_request(&cmd).unwrap(), &[])
            .await
            .unwrap();
        assert_eq!(v["name"], "p1");
        let s = seen.lock().unwrap()[0].clone();
        assert_eq!(s.method, "POST");
        assert_eq!(s.path, "/api/v1/enterprise/dr/pairs");
        assert_eq!(s.auth, "Bearer tok-123");
        let b: Value = serde_json::from_str(&s.body).unwrap();
        assert_eq!(
            b,
            json!({"name":"p1","vm":"web","rpo_seconds":900,"dr_namespace":"dr-ns"})
        );
    }

    #[test]
    fn dr_get_routes() {
        let r = dr_request(&dr_cmd(&["dr", "pair", "list"])).unwrap();
        assert_eq!(
            (r.method, r.path.as_str()),
            ("GET", "/api/v1/enterprise/dr/pairs")
        );
        let r = dr_request(&dr_cmd(&["dr", "pair", "status", "p1"])).unwrap();
        assert_eq!(r.path, "/api/v1/enterprise/dr/pairs/p1/status");
        let r = dr_request(&dr_cmd(&["dr", "pair", "failback-plan", "p1"])).unwrap();
        assert_eq!(r.path, "/api/v1/enterprise/dr/pairs/p1/failback/plan");
    }

    #[test]
    fn dr_failover_defaults_to_dry_run_and_needs_exact_phrase() {
        let r = dr_request(&dr_cmd(&["dr", "pair", "failover", "p1"])).unwrap();
        assert_eq!(r.body.unwrap(), json!({"dry_run": true}));

        // --execute without or with a wrong phrase builds no request at all.
        assert!(dr_request(&dr_cmd(&["dr", "pair", "failover", "p1", "--execute"])).is_err());
        assert!(dr_request(&dr_cmd(&[
            "dr",
            "pair",
            "failover",
            "p1",
            "--execute",
            "--confirm",
            "failover p2"
        ]))
        .is_err());
        // --confirm alone (no --execute) is refused rather than silently ignored.
        assert!(dr_request(&dr_cmd(&[
            "dr",
            "pair",
            "failover",
            "p1",
            "--confirm",
            "failover p1"
        ]))
        .is_err());

        let r = dr_request(&dr_cmd(&[
            "dr",
            "pair",
            "failover",
            "p1",
            "--execute",
            "--confirm",
            "failover p1",
            "--no-start",
        ]))
        .unwrap();
        assert_eq!(
            r.body.unwrap(),
            json!({"dry_run": false, "confirm": "failover p1", "start": false})
        );
    }

    #[test]
    fn dr_resume_and_validation() {
        assert!(dr_request(&dr_cmd(&[
            "dr",
            "pair",
            "resume",
            "p1",
            "--confirm",
            "nope"
        ]))
        .is_err());
        let r = dr_request(&dr_cmd(&[
            "dr",
            "pair",
            "resume",
            "p1",
            "--confirm",
            "resume p1",
        ]))
        .unwrap();
        assert_eq!(r.body.unwrap(), json!({"confirm": "resume p1"}));
        // bad rpo, bad name
        assert!(dr_request(&dr_cmd(&[
            "dr",
            "pair",
            "create",
            "p1",
            "--vm",
            "web",
            "--rpo-seconds",
            "5"
        ]))
        .is_err());
        assert!(dr_request(&DrPairCommands::Status {
            name: "../x".into()
        })
        .is_err());
    }

    #[test]
    fn volume_encryption_requests() {
        let r = volume_request(&ve_cmd(&["volume", "encryption", "status"]), "default").unwrap();
        assert_eq!(r.path, "/api/volume-encryption/status");
        let plan = ve_cmd(&[
            "--namespace",
            "prod",
            "volume",
            "encryption",
            "plan",
            "web",
            "data",
            "--size",
            "20Gi",
            "--storage-class",
            "ceph-enc",
        ]);
        let r = volume_request(&plan, "prod").unwrap();
        assert_eq!(r.path, "/api/volume-encryption/plan");
        assert_eq!(
            r.body.unwrap(),
            json!({"namespace":"prod","vm":"web","volume":"data","size":"20Gi",
                   "storage_class":"ceph-enc","mode":"storage_class",
                   "acknowledge_unverified_encryption":false})
        );
        let apply_args = [
            "volume",
            "encryption",
            "apply",
            "web",
            "data",
            "--size",
            "20Gi",
            "--storage-class",
            "c",
        ];
        // apply and rotate need an explicit --yes
        assert!(volume_request(&ve_cmd(&apply_args), "prod").is_err());
        let mut with_yes = apply_args.to_vec();
        with_yes.push("--yes");
        let r = volume_request(&ve_cmd(&with_yes), "prod").unwrap();
        assert_eq!(r.path, "/api/volume-encryption/apply");
        assert!(volume_request(
            &ve_cmd(&["volume", "encryption", "rotate", "web", "data"]),
            "p"
        )
        .is_err());
        let r = volume_request(
            &ve_cmd(&["volume", "encryption", "rotate", "web", "data", "--yes"]),
            "p",
        )
        .unwrap();
        assert_eq!(
            r.body.unwrap(),
            json!({"namespace":"p","vm":"web","volume":"data"})
        );
    }

    #[tokio::test]
    async fn experimental_disabled_is_reported_clearly() {
        let (url, _) = mock(
            403,
            json!({"success": false, "error": {"code": "EXPERIMENTAL_DISABLED",
                "message": "volume-encryption is Experimental; set ZORVIA_EXPERIMENTAL=1 to enable"}}),
        )
        .await;
        let e = remote(&url)
            .send(&get("/api/volume-encryption/status".into()), &[])
            .await
            .unwrap_err()
            .to_string();
        assert!(e.contains("Experimental and disabled"), "{e}");
        assert!(e.contains("ZORVIA_EXPERIMENTAL=1"), "{e}");
    }

    #[tokio::test]
    async fn other_errors_carry_status_and_hint() {
        let (url, _) = mock(
            403,
            json!({"error": {"code": "FORBIDDEN", "message": "no"}}),
        )
        .await;
        let e = remote(&url)
            .send(&get("/x".into()), &[])
            .await
            .unwrap_err()
            .to_string();
        assert!(e.contains("403") && e.contains("cluster.admin"), "{e}");
        let (url, _) = mock(
            401,
            json!({"error": {"code": "UNAUTHORIZED", "message": "bad"}}),
        )
        .await;
        let e = remote(&url)
            .send(&get("/x".into()), &[])
            .await
            .unwrap_err()
            .to_string();
        assert!(e.contains("ZORVIA_TOKEN"), "{e}");
    }

    #[test]
    fn rescue_has_no_password_argument() {
        // A password on argv must not parse at all.
        assert!(parse(&[
            "rescue",
            "reset-password",
            "vm1",
            "--user",
            "root",
            "--password",
            "hunter2xx"
        ])
        .is_err());
        assert!(parse(&[
            "rescue",
            "reset-password",
            "vm1",
            "--user",
            "root",
            "-p",
            "hunter2xx"
        ])
        .is_err());
        // and the subcommand itself takes no env-derived secret
        let cmd = rescue_cmd(&["rescue", "reset-password", "vm1", "--user", "root"]);
        assert!(!format!("{cmd:?}").contains("password:"));
    }

    #[test]
    fn password_from_reader_and_redacted_debug() {
        let mut r = std::io::Cursor::new(b"s3cret-Pass\r\nrest".to_vec());
        let p = read_password_line(&mut r).unwrap();
        assert_eq!(p.expose(), "s3cret-Pass");
        assert!(!format!("{p:?}").contains("s3cret"));
        assert!(read_password_line(&mut std::io::Cursor::new(Vec::<u8>::new())).is_err());
    }

    #[tokio::test]
    async fn rescue_password_only_in_body_and_scrubbed_from_errors() {
        // The server (maliciously or carelessly) echoes the password in its error message.
        let (url, seen) = mock(
            400,
            json!({"error": {"code": "INVALID", "message": "bad password hunter2-secret for root"}}),
        )
        .await;
        let cmd = rescue_cmd(&["rescue", "reset-password", "vm1", "--user", "root"]);
        let pw = Secret::new("hunter2-secret".into());
        let req = rescue_request(&cmd, Some(&pw)).unwrap();
        let err = remote(&url).send(&req, &[pw.expose()]).await.unwrap_err();
        let shown = format!("{err} {err:?}");
        assert!(!shown.contains("hunter2-secret"), "{shown}");
        assert!(shown.contains("***"));

        let s = seen.lock().unwrap()[0].clone();
        assert_eq!(s.path, "/api/vms/vm1/rescue");
        let b: Value = serde_json::from_str(&s.body).unwrap();
        assert_eq!(b["operation"], "reset-password");
        assert_eq!(b["password"], "hunter2-secret");
        // The built request is the only place the value lives; its Debug is the raw JSON, so
        // callers must not log it. Handlers never do.
    }

    #[test]
    fn rescue_other_ops() {
        let r = rescue_request(
            &rescue_cmd(&["rescue", "install-packages", "vm1", "curl", "git"]),
            None,
        )
        .unwrap();
        assert_eq!(
            r.body.unwrap(),
            json!({"operation":"install-packages","packages":["curl","git"]})
        );
        assert!(rescue_request(
            &rescue_cmd(&["rescue", "install-packages", "vm1", "a;rm"]),
            None
        )
        .is_err());
        let r = rescue_request(&rescue_cmd(&["rescue", "inspect", "vm1"]), None).unwrap();
        assert_eq!(r.body.unwrap(), json!({"operation":"inspect"}));
        let r = rescue_request(&rescue_cmd(&["rescue", "status", "vm1", "job-1"]), None).unwrap();
        assert_eq!(
            (r.method, r.path.as_str()),
            ("GET", "/api/vms/vm1/rescue/job-1")
        );
        // weak local validation: empty password rejected before any request
        let cmd = rescue_cmd(&["rescue", "reset-password", "vm1", "--user", "root"]);
        assert!(rescue_request(&cmd, Some(&Secret::new(String::new()))).is_err());
        assert!(rescue_request(&cmd, None).is_err());
    }

    #[test]
    fn rendering() {
        let v = json!({"a": 1, "b": {"c": "x"}, "l": [1, {"k": true}]});
        assert_eq!(
            serde_json::from_str::<Value>(&render(&v, "json").unwrap()).unwrap(),
            v
        );
        let t = render(&v, "table").unwrap();
        assert!(t.contains("a: 1") && t.contains("  c: x"));
        assert!(render(&v, "xml").is_err());
    }

    #[test]
    fn missing_url_or_token_is_a_clear_error() {
        let mut a = RemoteArgs {
            url: None,
            token: Some("t".into()),
            insecure: false,
            ca_cert: None,
            output: "json".into(),
        };
        assert!(Remote::new(&a)
            .err()
            .unwrap()
            .to_string()
            .contains("ZORVIA_URL"));
        a.url = Some("http://x".into());
        a.token = None;
        assert!(Remote::new(&a)
            .err()
            .unwrap()
            .to_string()
            .contains("ZORVIA_TOKEN"));
    }

    fn pool_cmd(args: &[&str]) -> PoolCommands {
        let mut v = vec!["inventory", "pool"];
        v.extend_from_slice(args);
        match *parse(&v).unwrap().command {
            Commands::Inventory(crate::cli::InventoryCommands::Pool { cmd, .. }) => cmd,
            _ => panic!("not a pool command"),
        }
    }

    #[test]
    fn pool_requests_map_to_the_api() {
        let r = pool_request(&pool_cmd(&["list"])).unwrap();
        assert_eq!(
            (r.method, r.path.as_str()),
            ("GET", "/api/v1/inventory/pools")
        );
        let r = pool_request(&pool_cmd(&["hierarchy"])).unwrap();
        assert_eq!(r.path, "/api/v1/inventory/pools/hierarchy");
        let r = pool_request(&pool_cmd(&["report", "web"])).unwrap();
        assert_eq!(r.path, "/api/v1/inventory/pools/web/report");
        let r = pool_request(&pool_cmd(&[
            "create",
            "web",
            "--max-vms",
            "5",
            "--max-memory-mib",
            "8192",
            "--description",
            "web tier",
        ]))
        .unwrap();
        assert_eq!(r.method, "POST");
        assert_eq!(
            r.body.unwrap(),
            json!({"name":"web","description":"web tier","max_vms":5,"max_memory_mib":8192})
        );
        let r = pool_request(&pool_cmd(&["assign", "web", "vm1", "--namespace", "ns"])).unwrap();
        assert_eq!(r.path, "/api/v1/inventory/pools/web/assign");
        assert_eq!(r.body.unwrap(), json!({"vm":"vm1","namespace":"ns"}));
        let r = pool_request(&pool_cmd(&["unassign", "web", "vm1"])).unwrap();
        assert_eq!(r.body.unwrap(), json!({"vm":"vm1","remove":true}));
        let r = pool_request(&pool_cmd(&["delete", "web", "--unassign"])).unwrap();
        assert_eq!(
            (r.method, r.path.as_str()),
            ("DELETE", "/api/v1/inventory/pools/web?unassign=true")
        );
        let r = pool_request(&pool_cmd(&["delete", "web"])).unwrap();
        assert_eq!(r.path, "/api/v1/inventory/pools/web");
    }

    #[test]
    fn pool_requests_reject_bad_input() {
        assert!(pool_request(&pool_cmd(&["show", "Bad/Name"])).is_err());
        assert!(pool_request(&pool_cmd(&["create", "web", "--max-vms", "0"])).is_err());
        assert!(pool_request(&pool_cmd(&["assign", "web", "../x"])).is_err());
    }

    #[tokio::test]
    async fn pool_delete_sends_delete_and_accepts_an_empty_204() {
        use axum::{http::StatusCode, routing::delete as del};
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let s2 = seen.clone();
        let app = axum::Router::new().route(
            "/api/v1/inventory/pools/web",
            del(move |m: axum::http::Method| async move {
                s2.lock().unwrap().push(m.to_string());
                StatusCode::NO_CONTENT
            }),
        );
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        tokio::spawn(async move {
            axum::serve(l, app).await.unwrap();
        });
        let v = remote(&url)
            .send(&pool_request(&pool_cmd(&["delete", "web"])).unwrap(), &[])
            .await
            .unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(seen.lock().unwrap().as_slice(), ["DELETE"]);
    }
}
