// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Rescue mode's Job entrypoint: mounts a VM's own disk image (via GuestKit,
//! which shells out to `losetup`/`qemu-nbd`/`mount`/`chroot` internally --
//! this binary needs to run in a privileged container) and applies one
//! offline guest-disk operation, then prints a single JSON result line to
//! stdout and exits 0/1. Zorvia's `rescue_handlers.rs` reads that line back
//! via the Job pod's logs, the same way pod-ops already fetches pod logs --
//! no separate result-passing infrastructure.
//!
//! All parameters arrive via environment variables (never CLI args built
//! from client input) set on the Job's container spec by
//! `src/kube/rescue.rs`. Only three operations are implemented -- see
//! docs/RESCUE.md for what's deferred and why.

use guestkit::Guestfs;
use serde::Serialize;
use std::env;
use std::io::Write;
use std::process::{Command, ExitCode, Stdio};

// Pure validation/redaction helpers shared with the server (and unit-tested
// there, since this binary needs the optional guestkit dependency).
#[path = "../kube/rescue_logic.rs"]
#[allow(dead_code)]
mod logic;

#[derive(Serialize)]
struct RescueResult {
    success: bool,
    operation: String,
    message: String,
    /// Bounded, redacted structured output (inspect only).
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<serde_json::Value>,
}

fn emit(mut result: RescueResult) -> ExitCode {
    // Nothing the agent says is trusted to be short or clean: errors from
    // guest tools can quote arbitrary guest content.
    result.message = logic::redact_text(&result.message, logic::MAX_TEXT_LEN * 4);
    // A single JSON line on stdout is the entire contract with
    // rescue_handlers.rs -- keep this the last thing printed.
    println!("{}", serde_json::to_string(&result).unwrap_or_default());
    if result.success {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn main() -> ExitCode {
    let operation = env::var("RESCUE_OPERATION").unwrap_or_default();
    let disk_path = env::var("RESCUE_DISK_PATH").unwrap_or_default();

    if disk_path.is_empty() {
        return emit(RescueResult {
            success: false,
            operation,
            message: "RESCUE_DISK_PATH is not set".into(),
            details: None,
        });
    }

    match run(&operation, &disk_path) {
        Ok((message, details)) => emit(RescueResult {
            success: true,
            operation,
            message,
            details,
        }),
        Err(e) => emit(RescueResult {
            success: false,
            operation,
            message: e.to_string(),
            details: None,
        }),
    }
}

fn run(operation: &str, disk_path: &str) -> anyhow::Result<(String, Option<serde_json::Value>)> {
    let mut g = Guestfs::builder().add_drive(disk_path).build_and_launch()?;

    // Same sequence GuestKit's own CLI uses to apply an offline plan
    // (cli/plan/apply.rs): find the OS root, mount every filesystem it
    // reports (parents before children), then operate. Refuse to guess at
    // a filesystem layout GuestKit itself couldn't identify.
    let roots = g.inspect_os()?;
    let root = roots
        .first()
        .ok_or_else(|| anyhow::anyhow!("no operating system found on this disk"))?;
    let mut mounts: Vec<(String, String)> = g.inspect_get_mountpoints(root)?.into_iter().collect();
    mounts.sort_by_key(|(mountpoint, _)| mountpoint.len());
    for (mountpoint, device) in &mounts {
        g.mount(device, mountpoint)?;
    }

    let mut details = None;
    let result = match operation {
        "set-hostname" => {
            let hostname = require_env("RESCUE_HOSTNAME")?;
            g.set_hostname(&hostname)?;
            format!("hostname set to '{hostname}'")
        }
        "reset-password" => {
            let user = require_env("RESCUE_USER")?;
            let user = logic::validate_username(&user).map_err(anyhow::Error::msg)?;
            reset_password(&mut g, user)?;
            format!("password reset for '{user}'")
        }
        "install-packages" => {
            let raw = require_env("RESCUE_PACKAGES")?;
            let list: Vec<String> = raw.split_whitespace().map(str::to_string).collect();
            let packages = logic::validate_packages(&list).map_err(anyhow::Error::msg)?;
            install_packages(&mut g, &packages)?
        }
        "inspect" => {
            let summary = inspect(&mut g, root)?;
            details = Some(summary);
            "inspection complete".to_string()
        }
        "inject-ssh-key" => {
            let user = require_env("RESCUE_SSH_USER")?;
            let key = require_env("RESCUE_SSH_KEY")?;
            g.set_ssh_authorized_keys(&user, &[key.as_str()])?;
            format!("authorized_keys replaced for '{user}'")
        }
        "enable-ssh" => {
            let unit = enable_ssh_unit(&mut g)?;
            format!("enabled '{unit}'")
        }
        other => anyhow::bail!("unsupported operation '{other}'"),
    };

    // autosync (default on) flushes on drop, but shutdown() surfaces any
    // unmount error instead of silently losing it.
    g.shutdown()?;
    Ok((result, details))
}

fn require_env(name: &str) -> anyhow::Result<String> {
    let value = env::var(name).unwrap_or_default();
    if value.trim().is_empty() {
        anyhow::bail!("{name} is not set");
    }
    Ok(value)
}

/// GuestKit has no direct "enable a systemd unit" mutator; its own CLI does
/// this with a plain symlink into multi-user.target.wants (identical logic
/// to `cli/plan/firstboot_stage.rs`'s `enable_unit`), no chroot needed.
/// Tries `ssh.service` then falls back to `sshd.service` for distros that
/// name the unit the other way.
fn enable_ssh_unit(g: &mut Guestfs) -> anyhow::Result<String> {
    const WANTS_DIR: &str = "/etc/systemd/system/multi-user.target.wants";
    g.mkdir_p(WANTS_DIR)?;
    for unit in ["ssh.service", "sshd.service"] {
        let link = format!("{WANTS_DIR}/{unit}");
        if g.ln_sf(&format!("../{unit}"), &link).is_ok()
            || g.ln_sf(&format!("/etc/systemd/system/{unit}"), &link)
                .is_ok()
        {
            return Ok(unit.to_string());
        }
    }
    anyhow::bail!("could not enable ssh.service or sshd.service")
}

/// Hashes a password with the host's `openssl passwd -6 -stdin` (SHA-512
/// crypt, random salt). The password is written to the child's stdin, never
/// to argv, env or disk.
fn hash_password(password: &str) -> anyhow::Result<String> {
    let mut child = Command::new("openssl")
        .args(["passwd", "-6", "-stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| anyhow::anyhow!("could not run openssl to hash the password"))?;
    child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("openssl stdin unavailable"))?
        .write_all(format!("{password}\n").as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        anyhow::bail!("openssl failed to hash the password");
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Replaces the user's hash in the guest's `/etc/shadow`. The file is
/// rewritten in place (same inode, so owner and mode are preserved). The
/// plain password is read from the mounted Secret file and never logged.
fn reset_password(g: &mut Guestfs, user: &str) -> anyhow::Result<()> {
    let path = require_env("RESCUE_PASSWORD_FILE")?;
    let password = std::fs::read_to_string(&path)
        .map_err(|_| anyhow::anyhow!("could not read the mounted password file"))?;
    logic::validate_password(&password).map_err(anyhow::Error::msg)?;
    let hash = hash_password(&password)?;
    drop(password);
    let shadow = g.cat("/etc/shadow")?;
    let updated = logic::set_shadow_hash(&shadow, user, &hash).map_err(anyhow::Error::msg)?;
    g.write("/etc/shadow", updated.as_bytes())?;
    Ok(())
}

/// Installs packages by chrooting into the guest and running its own package
/// manager as an explicit argv (no shell). The guest has no network of its
/// own while offline, so the Job pod's resolv.conf is placed temporarily in
/// the guest and the original restored afterwards. /proc, /dev and /sys are
/// bind-mounted into the guest root for the duration (and unmounted in
/// reverse order afterwards, even on error) so maintainer scripts work. The
/// Job pod needs outbound network access to the guest's package mirrors.
fn install_packages(g: &mut Guestfs, packages: &[String]) -> anyhow::Result<String> {
    let pm = logic::PackageManager::probes()
        .iter()
        .find(|(bin, _)| g.exists(bin).unwrap_or(false))
        .map(|(_, pm)| *pm)
        .ok_or_else(|| anyhow::anyhow!("no supported package manager found in the guest"))?;

    let original = g.read_file("/etc/resolv.conf").ok();
    let host_dns = std::fs::read("/etc/resolv.conf")
        .map_err(|_| anyhow::anyhow!("job pod has no /etc/resolv.conf"))?;
    g.write("/etc/resolv.conf", &host_dns)
        .map_err(|e| anyhow::anyhow!("could not provide DNS to the guest chroot: {e}"))?;

    // Guest "/" as seen from the Job pod: GuestKit records every mount's host
    // path; the shortest is the root (/boot, /boot/efi nest under it).
    let root = g
        .mountpoints()
        .ok()
        .and_then(|m| m.values().min_by_key(|p| p.len()).cloned());
    let outcome = match root {
        Some(root) => logic::with_chroot_binds(&mut ShellRunner, &root, |_| {
            for argv in pm.install_commands(packages) {
                let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
                g.command(&refs).map_err(|e| e.to_string())?;
            }
            Ok(())
        })
        .map_err(anyhow::Error::msg),
        None => Err(anyhow::anyhow!("could not locate the guest root mount")),
    };

    // Always restore the guest's resolver config.
    match original {
        Some(bytes) => {
            let _ = g.write("/etc/resolv.conf", &bytes);
        }
        None => {
            let _ = g.rm("/etc/resolv.conf");
        }
    }
    outcome?;
    Ok(format!(
        "installed {} package(s): {}",
        packages.len(),
        packages.join(", ")
    ))
}

/// Runs the mount/umount argvs built by `logic::with_chroot_binds` for real.
struct ShellRunner;

impl logic::CommandRunner for ShellRunner {
    fn run(&mut self, argv: &[String]) -> Result<(), String> {
        let (prog, args) = argv.split_first().ok_or("empty command")?;
        let out = Command::new(prog)
            .args(args)
            .output()
            .map_err(|e| format!("{prog}: {e}"))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
        }
    }
}

fn clip(s: &str) -> String {
    logic::redact_text(s, logic::MAX_TEXT_LEN)
}

/// Read-only summary of the guest: OS identity, mountpoints and a capped
/// application list. Every string is control-stripped, secret-masked and
/// length-capped; every list is capped; no file contents are read.
fn inspect(g: &mut Guestfs, root: &str) -> anyhow::Result<serde_json::Value> {
    let mut out = serde_json::Map::new();
    let mut put = |k: &str, v: anyhow::Result<String>| {
        if let Ok(v) = v {
            if !v.is_empty() {
                out.insert(k.to_string(), serde_json::Value::String(clip(&v)));
            }
        }
    };
    put("os_type", g.inspect_get_type(root).map_err(Into::into));
    put("distro", g.inspect_get_distro(root).map_err(Into::into));
    put(
        "product_name",
        g.inspect_get_product_name(root).map_err(Into::into),
    );
    put("arch", g.inspect_get_arch(root).map_err(Into::into));
    put("hostname", g.inspect_get_hostname(root).map_err(Into::into));
    put(
        "package_format",
        g.inspect_get_package_format(root).map_err(Into::into),
    );
    put(
        "init_system",
        g.inspect_get_init_system(root).map_err(Into::into),
    );
    if let (Ok(maj), Ok(min)) = (
        g.inspect_get_major_version(root),
        g.inspect_get_minor_version(root),
    ) {
        out.insert("version".into(), format!("{maj}.{min}").into());
    }
    if let Ok(mounts) = g.inspect_get_mountpoints(root) {
        let mut mp: Vec<String> = mounts.keys().map(|k| clip(k)).collect();
        mp.sort();
        mp.truncate(logic::MAX_LIST_ITEMS);
        out.insert("mountpoints".into(), mp.into());
    }
    if let Ok(apps) = g.inspect_list_applications(root) {
        let total = apps.len();
        let listed: Vec<String> = apps
            .iter()
            .take(logic::MAX_LIST_ITEMS)
            .map(|a| clip(&format!("{} {}", a.name, a.version).trim().to_string()))
            .collect();
        out.insert("applications_total".into(), total.into());
        out.insert("applications".into(), listed.into());
    }
    Ok(serde_json::Value::Object(out))
}
