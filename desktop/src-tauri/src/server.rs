//! Run the bundled `zorvia api-serve` and track its readiness.

use crate::{paths, secrets};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

const READY_TIMEOUT: Duration = Duration::from_secs(40);
const POLL_INTERVAL: Duration = Duration::from_millis(200);

pub enum ServerStatus {
    Ready { url: String },
    Failed(String),
}

struct ServerHandle {
    child: Child,
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Default)]
pub struct ServerState {
    status: Mutex<Option<ServerStatus>>,
    handle: Mutex<Option<ServerHandle>>,
}

impl ServerState {
    fn set_status(&self, status: ServerStatus) {
        if let Ok(mut g) = self.status.lock() {
            *g = Some(status);
        }
    }
}

fn free_port() -> Result<u16, String> {
    let l = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?;
    let port = l.local_addr().map_err(|e| e.to_string())?.port();
    drop(l);
    Ok(port)
}

/// True once `GET /api/health` answers 200 (the TCP port opening first is
/// not enough: the server binds before its Kubernetes client is ready).
fn healthy(port: u16) -> bool {
    let Ok(mut s) = TcpStream::connect_timeout(
        &([127, 0, 0, 1], port).into(),
        Duration::from_millis(500),
    ) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    if s
        .write_all(b"GET /api/health HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n")
        .is_err()
    {
        return false;
    }
    let mut buf = [0u8; 32];
    let n = s.read(&mut buf).unwrap_or(0);
    std::str::from_utf8(&buf[..n]).is_ok_and(|h| h.contains(" 200"))
}

fn spawn_serve(
    resource_dir: Option<PathBuf>,
    settings: &paths::AppSettings,
) -> Result<(Child, u16), String> {
    let bin = paths::resolve_zorvia_bin(resource_dir.clone());
    let web = paths::resolve_web_dir(resource_dir);
    let port = free_port()?;
    let data = paths::app_data_dir();
    std::fs::create_dir_all(&data).map_err(|e| e.to_string())?;

    let mut cmd = Command::new(&bin);
    cmd.args(["api-serve", "--host", "127.0.0.1", "--port", &port.to_string()])
        .current_dir(&data)
        .env("ZORVIA_WEB_DIR", &web)
        .env("ZORVIA_JWT_SECRET", secrets::jwt_secret()?)
        .env("ZORVIA_ADMIN_USER", "admin")
        .env("ZORVIA_ADMIN_PASSWORD", secrets::admin_password()?)
        .env("ZORVIA_AUTH_DB", data.join("auth.db"))
        .env("ZORVIA_AUDIT_DB", data.join("audit.db"))
        .env("ZORVIA_OPS_DB", data.join("ops.db"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(p) = paths::login_shell_path() {
        cmd.env("PATH", p);
    }
    if let Some(k) = settings.kubeconfig.as_deref().filter(|s| !s.trim().is_empty()) {
        cmd.env("KUBECONFIG", k);
    }
    if let Some(n) = settings.namespace.as_deref().filter(|s| !s.trim().is_empty()) {
        cmd.env("ZORVIA_NAMESPACE", n);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to launch {} ({e})", bin.display()))?;
    if let Some(o) = child.stdout.take() {
        thread::spawn(move || {
            for l in BufReader::new(o).lines().map_while(Result::ok) {
                println!("[zorvia] {l}");
            }
        });
    }
    if let Some(e) = child.stderr.take() {
        thread::spawn(move || {
            for l in BufReader::new(e).lines().map_while(Result::ok) {
                eprintln!("[zorvia] {l}");
            }
        });
    }
    Ok((child, port))
}

pub fn start_in_background(app: tauri::AppHandle, settings: paths::AppSettings) {
    thread::spawn(move || {
        use tauri::Manager;
        let state = app.state::<ServerState>();
        if let Some(url) = settings
            .remote_url
            .as_deref()
            .map(str::trim)
            .filter(|u| !u.is_empty())
        {
            state.set_status(ServerStatus::Ready { url: url.trim_end_matches('/').to_string() });
            return;
        }
        match spawn_serve(app.path().resource_dir().ok(), &settings) {
            Ok((mut child, port)) => {
                let start = Instant::now();
                while start.elapsed() < READY_TIMEOUT {
                    if let Ok(Some(status)) = child.try_wait() {
                        state.set_status(ServerStatus::Failed(format!(
                            "zorvia exited early ({status}). Is a kubeconfig available? Set one in Settings."
                        )));
                        return;
                    }
                    if healthy(port) {
                        if let Ok(mut g) = state.handle.lock() {
                            *g = Some(ServerHandle { child });
                        }
                        state.set_status(ServerStatus::Ready {
                            url: format!("http://127.0.0.1:{port}/"),
                        });
                        return;
                    }
                    thread::sleep(POLL_INTERVAL);
                }
                let _ = child.kill();
                state.set_status(ServerStatus::Failed(format!(
                    "zorvia did not become healthy on port {port} within {}s",
                    READY_TIMEOUT.as_secs()
                )));
            }
            Err(e) => state.set_status(ServerStatus::Failed(e)),
        }
    });
}

/// `Ok(None)` = starting, `Ok(Some(url))` = ready, `Err` = failed.
pub fn app_url(state: &ServerState) -> Result<Option<String>, String> {
    match state.status.lock().map_err(|_| "server state poisoned")?.as_ref() {
        None => Ok(None),
        Some(ServerStatus::Ready { url }) => Ok(Some(url.clone())),
        Some(ServerStatus::Failed(e)) => Err(e.clone()),
    }
}

pub fn shutdown(state: &ServerState) {
    if let Ok(mut g) = state.handle.lock() {
        *g = None;
    }
}
