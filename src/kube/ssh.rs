// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! In-browser SSH helpers: command construction and input validation.
//!
//! The web console proxies a local `ssh` or `virtctl ssh` process over
//! an authenticated WebSocket. These helpers are unit-tested without a cluster.

use anyhow::{bail, Result};

/// Allowed SSH user: 1-32 chars, starts with a letter or `_`, then [A-Za-z0-9._-].
pub fn validate_ssh_user(user: &str) -> Result<()> {
    if user.is_empty() || user.len() > 32 {
        bail!("SSH user must be 1-32 characters");
    }
    let mut chars = user.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_alphabetic() && first != '_' {
        bail!("SSH user must start with a letter or underscore");
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) {
        bail!("SSH user contains invalid characters");
    }
    Ok(())
}

pub fn validate_ssh_port(port: u16) -> Result<()> {
    if port == 0 {
        bail!("SSH port must be 1-65535");
    }
    Ok(())
}

/// argv for OpenSSH to a guest IP (lab-friendly host-key policy).
pub fn ssh_argv(user: &str, host: &str, port: u16) -> Result<Vec<String>> {
    validate_ssh_user(user)?;
    validate_ssh_port(port)?;
    if host.is_empty() {
        bail!("SSH host is empty");
    }
    if host
        .chars()
        .any(|c| c.is_whitespace() || matches!(c, ';' | '|' | '&' | '`' | '$'))
    {
        bail!("SSH host contains unsafe characters");
    }
    Ok(vec![
        "ssh".into(),
        "-tt".into(),
        "-o".into(),
        "StrictHostKeyChecking=accept-new".into(),
        "-o".into(),
        "UserKnownHostsFile=/tmp/zorvia-known-hosts".into(),
        "-p".into(),
        port.to_string(),
        format!("{user}@{host}"),
    ])
}

/// argv for virtctl when the guest has no reachable IP yet.
pub fn virtctl_ssh_argv(user: &str, vm: &str, namespace: &str) -> Result<Vec<String>> {
    validate_ssh_user(user)?;
    crate::kube::lifecycle::validate_k8s_name("name", vm)?;
    crate::kube::lifecycle::validate_k8s_name("namespace", namespace)?;
    Ok(vec![
        "virtctl".into(),
        "ssh".into(),
        format!("--user={user}"),
        vm.to_string(),
        "-n".into(),
        namespace.to_string(),
    ])
}

pub fn zorvia_ssh_ws_path(vm: &str, user: &str) -> Result<String> {
    crate::kube::lifecycle::validate_k8s_name("name", vm)?;
    validate_ssh_user(user)?;
    Ok(format!("/ws/ssh/{vm}?user={}", urlencoding_lite(user)))
}

fn urlencoding_lite(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' => c.to_string(),
            _ => format!("%{:02X}", c as u8),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_validation() {
        assert!(validate_ssh_user("zorvia").is_ok());
        assert!(validate_ssh_user("ubuntu").is_ok());
        assert!(validate_ssh_user("root").is_ok());
        assert!(validate_ssh_user("").is_err());
        assert!(validate_ssh_user("bad user").is_err());
        assert!(validate_ssh_user("x;rm").is_err());
    }

    #[test]
    fn ssh_argv_is_safe() {
        let argv = ssh_argv("ubuntu", "10.0.0.12", 22).unwrap();
        assert_eq!(argv[0], "ssh");
        assert!(argv.contains(&"ubuntu@10.0.0.12".to_string()));
        assert!(ssh_argv("ubuntu", "10.0.0.12;reboot", 22).is_err());
    }

    #[test]
    fn virtctl_argv() {
        let argv = virtctl_ssh_argv("cloud-user", "prod-db", "default").unwrap();
        assert_eq!(argv[0], "virtctl");
        assert!(argv.iter().any(|a| a == "--user=cloud-user"));
    }

    #[test]
    fn ws_path() {
        assert_eq!(
            zorvia_ssh_ws_path("web-01", "ubuntu").unwrap(),
            "/ws/ssh/web-01?user=ubuntu"
        );
    }
}
