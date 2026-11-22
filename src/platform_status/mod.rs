// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Cilium-style colorful platform status for `zorvia status` (no VM name).

pub mod ansi;
pub mod collect;
pub mod format;

pub use ansi::{strip_ansi, ErrorCount};
pub use collect::{collect, collect_default};
pub use format::{FeatureStatus, PlatformStatus, PodStateCount};

use anyhow::{bail, Result};
use std::io::{self, Write};
use std::time::{Duration, Instant};

/// Display platform status (summary or JSON), optionally waiting for readiness.
pub async fn display(
    output: &str,
    wait: bool,
    wait_duration: Duration,
    interactive: bool,
) -> Result<()> {
    let deadline = Instant::now() + wait_duration;
    let mut last_lines: usize = 0;

    loop {
        let status = match collect_default().await {
            Ok(s) => s,
            Err(e) => {
                let mut s = PlatformStatus::default();
                for key in ["kubevirt", "cdi", "zorvia-api", "snapshots", "rook"] {
                    s.components.insert(
                        key.into(),
                        ErrorCount {
                            disabled: true,
                            ..Default::default()
                        },
                    );
                }
                s.collection_errors
                    .push(format!("unable to connect to cluster: {e}"));
                s
            }
        };

        if output == "json" {
            println!("{}", serde_json::to_string_pretty(&status)?);
        } else {
            let text = status.format();
            if interactive && wait && last_lines > 0 {
                let mut stderr = io::stderr().lock();
                for _ in 0..last_lines {
                    let _ = write!(stderr, "\x1b[A\x1b[2K");
                }
                let _ = stderr.flush();
            }
            print!("{}", text);
            let _ = io::stdout().flush();
            last_lines = text.lines().count();
        }

        if !wait {
            // Soft failure only when we could not talk to the cluster at all
            if !status.collection_errors.is_empty()
                && status.components.values().all(|c| c.disabled)
            {
                bail!("unable to collect platform status");
            }
            return Ok(());
        }

        if status.is_ready() {
            return Ok(());
        }

        if Instant::now() >= deadline {
            bail!("timed out waiting for platform status to become ready");
        }

        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}
