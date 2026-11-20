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
use std::process::ExitCode;

#[derive(Serialize)]
struct RescueResult {
    success: bool,
    operation: String,
    message: String,
}

fn emit(result: RescueResult) -> ExitCode {
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
        });
    }

    match run(&operation, &disk_path) {
        Ok(message) => emit(RescueResult {
            success: true,
            operation,
            message,
        }),
        Err(e) => emit(RescueResult {
            success: false,
            operation,
            message: e.to_string(),
        }),
    }
}

fn run(operation: &str, disk_path: &str) -> anyhow::Result<String> {
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

    let result = match operation {
        "set-hostname" => {
            let hostname = require_env("RESCUE_HOSTNAME")?;
            g.set_hostname(&hostname)?;
            format!("hostname set to '{hostname}'")
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
    Ok(result)
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
