//! Admin credentials generated on first run and kept in the login Keychain.

use rand::distr::{Alphanumeric, SampleString};
use std::process::Command;

const SERVICE: &str = "dev.zyvor.zorvia.desktop";

fn read(account: &str) -> Option<String> {
    let out = Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", SERVICE, "-a", account, "-w"])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

fn write(account: &str, value: &str) -> Result<(), String> {
    let status = Command::new("/usr/bin/security")
        .args(["add-generic-password", "-U", "-s", SERVICE, "-a", account, "-w", value])
        .status()
        .map_err(|e| e.to_string())?;
    status.success().then_some(()).ok_or_else(|| "keychain write failed".into())
}

/// Returns the stored secret, creating a random 32-char one on first use.
pub fn get_or_create(account: &str) -> Result<String, String> {
    if let Some(v) = read(account) {
        return Ok(v);
    }
    let v = Alphanumeric.sample_string(&mut rand::rng(), 32);
    write(account, &v)?;
    Ok(v)
}

pub fn admin_password() -> Result<String, String> {
    get_or_create("admin-password")
}

pub fn jwt_secret() -> Result<String, String> {
    get_or_create("jwt-secret")
}
