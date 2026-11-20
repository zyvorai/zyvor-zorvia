// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Off-cluster backup Job entrypoint: streams the VM disk images mounted
//! read-only under /disks/ to S3-compatible storage (see
//! `zorvia::backup::agent`). Configuration arrives via environment
//! variables set on the Job by `src/kube/backup_job.rs` (`BACKUP_MODE=restore`
//! selects the restore direction: read a backup back onto /restore/ mounts); credentials and the
//! encryption key come from Kubernetes Secrets, never from arguments.
//!
//! stdout is a stream of JSON lines. Progress lines look like
//! `{"progress":42,"phase":"uploading root"}`; the last line is the result:
//! `{"success":true,...}` or `{"success":false,"error":"..."}`. Zorvia reads
//! them back from the Job pod's logs. Exits 0 on success, 1 on failure.

use std::process::ExitCode;
use zorvia::backup::agent::{
    run, run_restore, run_verify_key, AgentConfig, RestoreConfig, VerifyKeyConfig,
};
use zorvia::backup::s3::S3Client;

fn line(v: serde_json::Value) {
    println!("{v}");
}

fn fail(e: impl std::fmt::Display) -> ExitCode {
    line(serde_json::json!({"success": false, "error": e.to_string()}));
    ExitCode::FAILURE
}

/// `BACKUP_MODE=restore` reads a backup back onto writable PVC mounts.
async fn restore_main() -> ExitCode {
    let cfg = match RestoreConfig::from_env(|k| std::env::var(k).ok()) {
        Ok(c) => c,
        Err(e) => return fail(format!("{e:#}")),
    };
    let client = match S3Client::new(cfg.s3.clone()) {
        Ok(c) => c,
        Err(e) => return fail(format!("{e:#}")),
    };
    let progress =
        |pct: u8, phase: &str| line(serde_json::json!({"progress": pct, "phase": phase}));
    match run_restore(&cfg, &client, &progress).await {
        Ok(result) => {
            let mut v = serde_json::to_value(&result).unwrap_or_default();
            v["success"] = serde_json::json!(true);
            line(v);
            ExitCode::SUCCESS
        }
        Err(e) => fail(format!("{e:#}")),
    }
}

/// `BACKUP_MODE=verify-key` checks that the supplied key opens a backup (first
/// part of each disk) without restoring anything or mounting any volume.
async fn verify_key_main() -> ExitCode {
    let cfg = match VerifyKeyConfig::from_env(|k| std::env::var(k).ok()) {
        Ok(c) => c,
        Err(e) => return fail(format!("{e:#}")),
    };
    let client = match S3Client::new(cfg.s3.clone()) {
        Ok(c) => c,
        Err(e) => return fail(format!("{e:#}")),
    };
    let progress =
        |pct: u8, phase: &str| line(serde_json::json!({"progress": pct, "phase": phase}));
    match run_verify_key(&cfg, &client, &progress).await {
        Ok(result) => {
            let mut v = serde_json::to_value(&result).unwrap_or_default();
            v["success"] = serde_json::json!(true);
            line(v);
            ExitCode::SUCCESS
        }
        Err(e) => fail(format!("{e:#}")),
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    match std::env::var("BACKUP_MODE").as_deref() {
        Ok("restore") => return restore_main().await,
        Ok("verify-key") => return verify_key_main().await,
        _ => {}
    }
    let cfg = match AgentConfig::from_env(|k| std::env::var(k).ok()) {
        Ok(c) => c,
        Err(e) => {
            line(serde_json::json!({"success": false, "error": format!("{e:#}")}));
            return ExitCode::FAILURE;
        }
    };
    let client = match S3Client::new(cfg.s3.clone()) {
        Ok(c) => c,
        Err(e) => {
            line(serde_json::json!({"success": false, "error": format!("{e:#}")}));
            return ExitCode::FAILURE;
        }
    };
    let progress =
        |pct: u8, phase: &str| line(serde_json::json!({"progress": pct, "phase": phase}));

    match run(&cfg, &client, &progress).await {
        Ok(result) => {
            let mut v = serde_json::to_value(&result).unwrap_or_default();
            v["success"] = serde_json::json!(true);
            line(v);
            ExitCode::SUCCESS
        }
        Err(e) => {
            line(serde_json::json!({"success": false, "error": format!("{e:#}")}));
            ExitCode::FAILURE
        }
    }
}
