// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

use crate::guest_insight::{GuestAgentState, GuestInsightReport};
use crate::kube::KubeClient;
use crate::tui::colors::cli as color;
use anyhow::{anyhow, Context, Result};

pub async fn handle_guest_insight(
    vm: String,
    output: String,
    strict: bool,
    namespace: &str,
    kubeconfig: Option<&str>,
) -> Result<()> {
    let client = match kubeconfig {
        Some(path) => KubeClient::with_kubeconfig(path)
            .await
            .with_context(|| format!("failed to load kubeconfig '{}'", path))?,
        None => KubeClient::new()
            .await
            .context("failed to create Kubernetes client")?,
    };

    let status = match client.get_vmi(namespace, &vm).await {
        Ok(vmi) => vmi.status,
        Err(error) => {
            let not_found = error.downcast_ref::<kube::Error>().is_some_and(
                |kube_error| matches!(kube_error, kube::Error::Api(api) if api.code == 404),
            );
            if not_found {
                None
            } else {
                return Err(error).with_context(|| {
                    format!("failed to get VirtualMachineInstance {}/{}", namespace, vm)
                });
            }
        }
    };

    let report = GuestInsightReport::from_status(&vm, namespace, status.as_ref());
    render_guest_insight(&report, &output)?;

    if strict && report.agent_state != GuestAgentState::Connected {
        return Err(anyhow!(
            "guest-insight strict gate failed: QEMU Guest Agent is {}",
            report.agent_state
        ));
    }

    Ok(())
}

fn render_guest_insight(report: &GuestInsightReport, output: &str) -> Result<()> {
    match output.to_ascii_lowercase().as_str() {
        "json" => println!("{}", serde_json::to_string_pretty(report)?),
        "yaml" | "yml" => println!("{}", serde_yaml::to_string(report)?),
        "table" | "text" => render_guest_table(report),
        other => {
            return Err(anyhow!(
                "unsupported guest-insight output '{}'; expected table, json, or yaml",
                other
            ))
        }
    }
    Ok(())
}

fn render_guest_table(report: &GuestInsightReport) {
    println!("{}", color::header("Zorvia Guest Insight"));
    println!();
    println!("  VM:          {}/{}", report.namespace, report.vm);
    println!("  Phase:       {}", report.phase);
    println!("  Node:        {}", report.node.as_deref().unwrap_or("-"));
    println!("  Guest Agent: {}", report.agent_state);
    println!("  Readiness:   {}/100", report.readiness_score);
    println!(
        "  Guest OS:    {} {}",
        report.os_name.as_deref().unwrap_or("-"),
        report.os_version.as_deref().unwrap_or("")
    );
    println!(
        "  Kernel:      {}",
        report.kernel_release.as_deref().unwrap_or("-")
    );
    println!();

    if !report.interfaces.is_empty() {
        println!("{}", color::label("Interfaces:"));
        for iface in &report.interfaces {
            let name = iface
                .guest_name
                .as_deref()
                .or(iface.name.as_deref())
                .unwrap_or("unknown");
            println!(
                "  - {:<14} {:<18} {}",
                name,
                iface.mac.as_deref().unwrap_or("-"),
                iface.primary_ip.as_deref().unwrap_or("-")
            );
        }
    }

    if !report.recommendations.is_empty() {
        println!();
        println!("{}", color::label("Recommendations:"));
        for recommendation in &report.recommendations {
            println!("  - {}", recommendation);
        }
    }

    println!();
    if report.healthy() {
        println!("{}", color::success("✓ Guest integration is healthy"));
    } else {
        println!("{}", color::warning("Guest integration needs attention"));
    }
}
