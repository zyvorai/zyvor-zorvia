// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

use crate::adopt::{build_report, render_table, VmSummary};
use crate::kube::KubeClient;
use anyhow::{anyhow, Context, Result};

/// `zorvia adopt`: read-only report on the VMs Zorvia can see and which
/// Zorvia capabilities apply today.
pub async fn handle_adopt(all_namespaces: bool, output: String, namespace: &str) -> Result<()> {
    let client = KubeClient::new()
        .await
        .context("failed to create Kubernetes client")?;

    let vms = if all_namespaces {
        client.list_all_vms().await?
    } else {
        client.list_vms(namespace).await?
    };

    let summaries: Vec<VmSummary> = vms.iter().map(VmSummary::from_vm).collect();
    let report = build_report(&summaries);

    match output.to_lowercase().as_str() {
        "json" => println!("{}", serde_json::to_string_pretty(&report)?),
        "yaml" | "yml" => println!("{}", serde_yaml::to_string(&report)?),
        "table" | "text" => print!("{}", render_table(&report)),
        other => {
            return Err(anyhow!(
                "unsupported adopt output '{}'; expected table, json, or yaml",
                other
            ))
        }
    }
    Ok(())
}
