//! # Zorvia
//!
//! Craft VMs for KubeVirt with Rust power.
//!
//! Zorvia provides a CLI and library for managing KubeVirt virtual machines
//! on Kubernetes clusters. It handles VM configuration, template management,
//! snapshots, backups, monitoring, and more.
//!
//! ## Quick Start (Library Usage)
//!
//! ```
//! use zorvia::{VMConfigBuilder, vm_config_to_kubevirt, to_yaml};
//!
//! // Build a VM configuration
//! let config = VMConfigBuilder::new("web-server")
//!     .namespace("production")
//!     .cpu(4, 1, 1)
//!     .memory("8Gi")
//!     .add_blank_disk("rootdisk", "40Gi", 1)
//!     .add_pod_network("eth0")
//!     .label("app", "nginx")
//!     .build();
//!
//! // Convert to KubeVirt manifest
//! let vm = vm_config_to_kubevirt(&config).unwrap();
//!
//! // Serialize to YAML
//! let yaml = to_yaml(&config).unwrap();
//! assert!(yaml.contains("web-server"));
//! ```

#[cfg(feature = "web")]
pub mod ai;
#[cfg(feature = "web")]
pub mod atlas;
pub mod cli;
#[cfg(feature = "web")]
pub mod commercial;
pub mod config;
#[cfg(feature = "web")]
pub mod keys;
#[cfg(feature = "web")]
pub mod kryton;
pub mod kube;
#[cfg(feature = "web")]
pub mod migration_import;
pub mod network;
#[cfg(feature = "web")]
pub mod operations;
pub mod output;
pub mod storage;
#[cfg(feature = "web")]
pub mod store;
pub mod templates;
pub mod terraform;
pub mod tui;
pub mod utils;

// Innovative features
pub mod adopt;
pub mod aiml;
pub mod api;
pub mod automation;
pub mod backup;
pub mod blueprints;
pub mod capacity;
pub mod change_plan;
pub mod compliance;
pub mod cost;
pub mod devexp;
pub mod devices;
pub mod disk;
pub mod dr;
pub mod edge;
pub mod enterprise;
pub mod features;
pub mod finops;
pub mod gitops;
pub mod golden_images;
pub mod guest_agent;
pub mod guest_insight;
#[cfg(feature = "web")]
pub mod guest_rpc;
pub mod handlers;
pub mod health;
pub mod migration;
pub mod monitoring;
pub mod multicloud;
pub mod multitenancy;
pub mod networking;
pub mod observability;
pub mod platform_status;
pub mod profiles;
pub mod secrets;
pub mod security;
pub mod servicemesh;
pub mod snapshots;

// Features ported from v9s
pub mod advanced_filter;
pub mod ai_troubleshoot;
pub mod audit_trail;
pub mod autoscaler;
pub mod change_approval;
pub mod cluster_health;
pub mod console_panel;
pub mod dependency_graph;
pub mod disk_conversion;
pub mod hypervisor;
pub mod macros;
pub mod multi_cluster;
pub mod nlp_search;
pub mod notifications;
pub mod placement;
pub mod power_schedule;
pub mod recommendation;
pub mod rook;
pub mod search_history;
pub mod session_sharing;
pub mod state_docs;
pub mod state_persistence;
pub mod topology;
pub mod vcenter_ops;
pub mod warm_pool;

// Convenience re-exports for public API
pub use config::{VMConfig, VMConfigBuilder};
pub use kube::converter::vm_config_to_kubevirt;
pub use output::{format_output, to_json, to_yaml, OutputFormat};
pub use utils::{format_bytes, generate_id, percent_to_u8, ZorviaError};

use anyhow::Result;
use cli::{
    AdvisorCommands, ApiCommands, AutomationCommands, BackupCommands, BlueprintCommands,
    ChangeCommands, Cli, Commands, ConfigCommands, CostCommands, DevCommands, DiskCommands,
    GuestCommands, HaCommands, InventoryCommands, MaintenanceCommands, MigrationCommands,
    MonitorCommands, NetworkCommands, ObservabilityCommands, PlacementCommands, ProfileCommands,
    SecurityCommands, SnapshotCommands, TemplateCommands, TenancyCommands, VmCommands,
};
use config::AppConfig;

/// Main entry point for the library
pub async fn run(mut cli: Cli) -> Result<()> {
    // Load application config file
    let app_config = if let Some(ref config_path) = cli.config {
        match AppConfig::load_from(std::path::PathBuf::from(config_path)) {
            Ok(c) => c,
            Err(e) => {
                eprintln!(
                    "Warning: Failed to load config file '{}': {}",
                    config_path, e
                );
                AppConfig::default()
            }
        }
    } else {
        AppConfig::load().unwrap_or_default()
    };

    // Apply config file defaults where CLI didn't override.
    // We detect "not explicitly set" by checking against clap's default_value.
    if cli.kubeconfig.is_none() {
        if let Some(ref kc) = app_config.kubeconfig {
            cli.kubeconfig = Some(kc.clone());
        }
    }
    // Only apply config namespace if CLI still has the clap default
    if cli.namespace == "default" && app_config.namespace != "default" {
        cli.namespace = app_config.namespace.clone();
    }

    // Initialize logging based on config + CLI
    let log_level = if cli.verbose {
        log::LevelFilter::Debug
    } else {
        match app_config.logging.level.as_str() {
            "error" => log::LevelFilter::Error,
            "warn" => log::LevelFilter::Warn,
            "debug" => log::LevelFilter::Debug,
            "trace" => log::LevelFilter::Trace,
            _ => log::LevelFilter::Info,
        }
    };

    env_logger::Builder::from_default_env()
        .filter_level(log_level)
        .try_init()
        .ok();

    match *cli.command {
        Commands::Vm(cmd) => match cmd {
            VmCommands::Create {
                name,
                template,
                from_file,
                cpus,
                memory,
                disk_size,
                storage_class,
                container_disk,
                cloud_init,
                dry_run,
                output,
            } => {
                handlers::vm::handle_create(
                    name,
                    template,
                    from_file,
                    cpus,
                    memory,
                    disk_size,
                    storage_class,
                    container_disk,
                    cloud_init,
                    dry_run,
                    output,
                    &cli.namespace,
                )
                .await?;
            }
            VmCommands::List {
                all_namespaces,
                output,
            } => {
                handlers::vm::handle_list(all_namespaces, output, &cli.namespace).await?;
            }
            VmCommands::Get { name, output } => {
                handlers::vm::handle_get(name, output, &cli.namespace).await?;
            }
            VmCommands::Delete { name, yes } => {
                handlers::vm::handle_delete(name, yes, &cli.namespace).await?;
            }
            VmCommands::Start { name } => {
                handlers::vm::handle_start(name, &cli.namespace).await?;
            }
            VmCommands::Stop { name } => {
                handlers::vm::handle_stop(name, &cli.namespace).await?;
            }
            VmCommands::Restart { name } => {
                handlers::vm::handle_restart(name, &cli.namespace).await?;
            }
            VmCommands::Pause { name } => {
                handlers::vm::handle_pause(name, &cli.namespace).await?;
            }
            VmCommands::Resume { name } => {
                handlers::vm::handle_resume(name, &cli.namespace).await?;
            }
            VmCommands::Console { name } => {
                handlers::vm::handle_console(name, &cli.namespace).await?;
            }
            VmCommands::Ssh { name, user } => {
                handlers::vm::handle_ssh(name, user, &cli.namespace).await?;
            }
            VmCommands::Vnc { name } => {
                handlers::vm::handle_vnc(name, &cli.namespace).await?;
            }
            VmCommands::Logs { name, follow, tail } => {
                handlers::vm::handle_logs(name, follow, tail, &cli.namespace).await?;
            }
            VmCommands::Status {
                name,
                watch,
                interval,
                output,
                wait,
                wait_duration,
                interactive,
            } => match name {
                None => {
                    platform_status::display(&output, wait, wait_duration, interactive).await?;
                }
                Some(name) => {
                    handlers::vm::handle_status(name, watch, interval, &cli.namespace).await?;
                }
            },
            VmCommands::Clone {
                source,
                target,
                start,
            } => {
                handlers::vm::handle_clone(source, target, start, &cli.namespace).await?;
            }
            VmCommands::Resources {
                all_namespaces,
                sort_by,
            } => {
                handlers::vm::handle_resources(all_namespaces, sort_by, &cli.namespace).await?;
            }
            VmCommands::Export {
                name,
                output,
                kubevirt,
            } => {
                handlers::vm::handle_export(name, output, kubevirt, &cli.namespace).await?;
            }
            VmCommands::Wizard { name } => {
                handlers::vm::handle_wizard(name, &cli.namespace).await?;
            }
            VmCommands::Batch {
                file,
                namespace,
                dry_run,
                continue_on_error,
            } => {
                handlers::vm::handle_batch(
                    file,
                    namespace,
                    dry_run,
                    continue_on_error,
                    &cli.namespace,
                )
                .await?;
            }
        },

        Commands::Template(cmd) => match cmd {
            TemplateCommands::Generate {
                name,
                template,
                from_file,
                cpus,
                memory,
                disk_size,
                output,
                format,
                kubevirt,
            } => {
                handlers::vm::handle_generate(
                    name,
                    template,
                    from_file,
                    cpus,
                    memory,
                    disk_size,
                    output,
                    format,
                    kubevirt,
                    &cli.namespace,
                )?;
            }
            TemplateCommands::Templates => {
                handlers::vm::handle_templates()?;
            }
            TemplateCommands::Images { output } => {
                handlers::vm::handle_images(output)?;
            }
            TemplateCommands::Template { name, output } => {
                handlers::vm::handle_template(name, output)?;
            }
            TemplateCommands::Validate { file } => {
                handlers::vm::handle_validate(file)?;
            }
        },

        Commands::Profile(cmd) => match cmd {
            ProfileCommands::Profiles { details } => {
                handlers::profiles::handle_profiles(details)?;
            }
            ProfileCommands::Profile { name, output } => {
                handlers::profiles::handle_profile(name, output)?;
            }
            ProfileCommands::ProfileCreate {
                name,
                cpus,
                sockets,
                threads,
                memory,
                disk_size,
                description,
                use_cases,
                recommended_os,
                from_file,
            } => {
                handlers::profiles::handle_profile_create(
                    name,
                    cpus,
                    sockets,
                    threads,
                    memory,
                    disk_size,
                    description,
                    use_cases,
                    recommended_os,
                    from_file,
                )?;
            }
            ProfileCommands::ProfileEdit {
                name,
                cpus,
                sockets,
                threads,
                memory,
                disk_size,
                description,
                use_cases,
                recommended_os,
            } => {
                handlers::profiles::handle_profile_edit(
                    name,
                    cpus,
                    sockets,
                    threads,
                    memory,
                    disk_size,
                    description,
                    use_cases,
                    recommended_os,
                )?;
            }
            ProfileCommands::ProfileDelete { name, yes } => {
                handlers::profiles::handle_profile_delete(name, yes)?;
            }
        },

        Commands::Blueprint(cmd) => match cmd {
            BlueprintCommands::Blueprints { tag, details } => {
                handlers::profiles::handle_blueprints(tag, details)?;
            }
            BlueprintCommands::Blueprint { name, output } => {
                handlers::profiles::handle_blueprint(name, output)?;
            }
            BlueprintCommands::Deploy {
                blueprint,
                prefix,
                start,
                dry_run,
            } => {
                handlers::profiles::handle_deploy(
                    blueprint,
                    prefix,
                    start,
                    dry_run,
                    cli.namespace.clone(),
                )
                .await?;
            }
            BlueprintCommands::BlueprintCreate {
                name,
                from_file,
                description,
            } => {
                handlers::profiles::handle_blueprint_create(name, from_file, description)?;
            }
            BlueprintCommands::BlueprintEdit { name, description } => {
                handlers::profiles::handle_blueprint_edit(name, description)?;
            }
            BlueprintCommands::BlueprintDelete { name, yes } => {
                handlers::profiles::handle_blueprint_delete(name, yes)?;
            }
            BlueprintCommands::BlueprintValidate { file, detailed } => {
                handlers::profiles::handle_blueprint_validate(file, detailed)?;
            }
        },

        Commands::Advisor(cmd) => match cmd {
            AdvisorCommands::Health { target, detailed } => {
                handlers::profiles::handle_health(target, detailed, cli.namespace.clone()).await?;
            }
            AdvisorCommands::Recommend {
                workload,
                alternatives,
            } => {
                handlers::profiles::handle_recommend(workload, alternatives)?;
            }
        },

        Commands::Snapshot(cmd) => match cmd {
            SnapshotCommands::SnapshotCreate {
                vm,
                name,
                description,
            } => {
                handlers::infra::handle_snapshot_create(vm, name, description, &cli.namespace)
                    .await?
            }
            SnapshotCommands::SnapshotList {
                vm,
                all_namespaces,
                output,
            } => {
                handlers::infra::handle_snapshot_list(vm, all_namespaces, output, &cli.namespace)
                    .await?
            }
            SnapshotCommands::SnapshotGet { name, output } => {
                handlers::infra::handle_snapshot_get(name, output, &cli.namespace).await?
            }
            SnapshotCommands::SnapshotDelete { name, yes } => {
                handlers::infra::handle_snapshot_delete(name, yes, &cli.namespace).await?
            }
            SnapshotCommands::SnapshotRestore {
                snapshot,
                target,
                in_place,
                start,
            } => {
                handlers::infra::handle_snapshot_restore(
                    snapshot,
                    target,
                    in_place,
                    start,
                    &cli.namespace,
                )
                .await?
            }
        },

        Commands::Monitor(cmd) => match cmd {
            MonitorCommands::MonitorLive { vm, interval } => {
                handlers::infra::handle_monitor_live(vm, interval, &cli.namespace).await?
            }
            MonitorCommands::MonitorStats { vm, period, output } => {
                handlers::infra::handle_monitor_stats(vm, period, output, &cli.namespace).await?
            }
            MonitorCommands::MonitorCompare { vms, output } => {
                handlers::infra::handle_monitor_compare(vms, output, &cli.namespace).await?
            }
            MonitorCommands::MonitorTop {
                all_namespaces,
                sort_by,
                limit,
            } => {
                handlers::infra::handle_monitor_top(all_namespaces, sort_by, limit, &cli.namespace)
                    .await?
            }
        },

        Commands::Disk(cmd) => match cmd {
            DiskCommands::DiskExpand {
                vm,
                disk,
                size,
                pvc,
                plan,
            } => {
                handlers::infra::handle_disk_expand(vm, disk, size, pvc, plan, &cli.namespace)
                    .await?
            }
            DiskCommands::DiskHealth { vm, detailed } => {
                handlers::infra::handle_disk_health(vm, detailed, &cli.namespace).await?
            }
            DiskCommands::DiskScript {
                filesystem,
                device,
                output,
                dry_run,
            } => handlers::infra::handle_disk_script(filesystem, device, output, dry_run)?,
            DiskCommands::DiskUsage {
                vm,
                sort_by,
                output,
            } => handlers::infra::handle_disk_usage(vm, sort_by, output, &cli.namespace).await?,
        },

        Commands::Network(cmd) => match cmd {
            NetworkCommands::NetworkList { vm, output } => {
                handlers::infra::handle_network_list(vm, output, &cli.namespace).await?
            }
            NetworkCommands::NetworkGet {
                vm,
                interface,
                output,
            } => handlers::infra::handle_network_get(vm, interface, output, &cli.namespace).await?,
            NetworkCommands::NetworkBandwidth {
                vm,
                interface,
                watch,
                interval,
            } => {
                handlers::infra::handle_network_bandwidth(
                    vm,
                    interface,
                    watch,
                    interval,
                    &cli.namespace,
                )
                .await?
            }
            NetworkCommands::NetworkTraffic {
                vm,
                interface,
                period,
                top,
                output,
            } => handlers::infra::handle_network_traffic(vm, interface, period, top, output)?,
            NetworkCommands::NetworkPolicies {
                all_namespaces,
                output,
            } => {
                handlers::infra::handle_network_policies(all_namespaces, output, &cli.namespace)
                    .await?
            }
            NetworkCommands::NetworkPolicy { name, output } => {
                handlers::infra::handle_network_policy(name, output)?
            }
        },

        Commands::Migration(cmd) => match cmd {
            MigrationCommands::Migrate {
                vm,
                target_node,
                migration_type,
                plan,
            } => {
                handlers::backup::handle_migrate(
                    vm,
                    target_node,
                    migration_type,
                    plan,
                    &cli.namespace,
                )
                .await?;
            }
            MigrationCommands::MigrationStatus {
                vm,
                watch,
                interval,
            } => {
                handlers::backup::handle_migration_status(vm, watch, interval, &cli.namespace)
                    .await?;
            }
            MigrationCommands::MigrationList {
                all_namespaces,
                state,
                output,
            } => {
                handlers::backup::handle_migration_list(
                    all_namespaces,
                    state,
                    output,
                    &cli.namespace,
                )
                .await?;
            }
        },

        Commands::Ha(cmd) => match cmd {
            HaCommands::HAConfig {
                vm,
                enable,
                disable,
                priority,
                eviction_strategy,
            } => {
                handlers::backup::handle_ha_config(
                    vm,
                    enable,
                    disable,
                    priority,
                    eviction_strategy,
                    &cli.namespace,
                )
                .await?;
            }
            HaCommands::HAStatus { vm, output } => {
                handlers::backup::handle_ha_status(vm, output, &cli.namespace)?;
            }
            HaCommands::EvacuateNode {
                node,
                reason,
                max_parallel,
                timeout,
                force,
                plan,
            } => {
                handlers::backup::handle_evacuate_node(
                    node,
                    reason,
                    max_parallel,
                    timeout,
                    force,
                    plan,
                    &cli.namespace,
                )
                .await?;
            }
            HaCommands::EvacuationStatus { node, watch } => {
                handlers::backup::handle_evacuation_status(node, watch, &cli.namespace).await?;
            }
        },

        Commands::Backup(cmd) => match cmd {
            BackupCommands::BackupCreate {
                vm,
                name,
                backup_type,
                compression,
                no_encryption,
            } => {
                handlers::backup::handle_backup_create(
                    vm,
                    name,
                    backup_type,
                    compression,
                    no_encryption,
                    &cli.namespace,
                )
                .await?;
            }
            BackupCommands::BackupList { vm, output } => {
                handlers::backup::handle_backup_list(vm, output, &cli.namespace).await?;
            }
            BackupCommands::BackupGet { name, output } => {
                handlers::backup::handle_backup_get(name, output, &cli.namespace)?;
            }
            BackupCommands::BackupDelete { name, yes } => {
                handlers::backup::handle_backup_delete(name, yes, &cli.namespace).await?;
            }
            BackupCommands::BackupRestore {
                backup,
                target,
                start,
            } => {
                handlers::backup::handle_backup_restore(backup, target, start, &cli.namespace)
                    .await?;
            }
            BackupCommands::BackupVerify {
                name,
                verification_type,
            } => {
                handlers::backup::handle_backup_verify(name, verification_type, &cli.namespace)
                    .await?;
            }
            BackupCommands::BackupSchedules { output } => {
                handlers::backup::handle_backup_schedules(output, &cli.namespace)?;
            }
            BackupCommands::BackupScheduleCreate { name, schedule, vm } => {
                handlers::backup::handle_backup_schedule_create(
                    name,
                    schedule,
                    vm,
                    &cli.namespace,
                )?;
            }
            BackupCommands::RecoveryPlan { name, output } => {
                handlers::backup::handle_recovery_plan(name, output, &cli.namespace)?;
            }
            BackupCommands::RecoveryExecute { plan, dry_run } => {
                handlers::backup::handle_recovery_execute(plan, dry_run, &cli.namespace).await?;
            }
        },

        Commands::Security(cmd) => match cmd {
            SecurityCommands::SecurityScan {
                vm,
                scan_type,
                containers,
                output,
            } => {
                handlers::security::handle_security_scan(
                    vm,
                    scan_type,
                    containers,
                    output,
                    &cli.namespace,
                )?;
            }
            SecurityCommands::SecurityAssess { vm, output } => {
                handlers::security::handle_security_assess(vm, output, &cli.namespace)?;
            }
            SecurityCommands::SecurityHarden {
                vm,
                profile,
                verify_only,
            } => {
                handlers::security::handle_security_harden(
                    vm,
                    profile,
                    verify_only,
                    &cli.namespace,
                )?;
            }
            SecurityCommands::SecurityProfiles { details } => {
                handlers::security::handle_security_profiles(details)?;
            }
            SecurityCommands::ComplianceCheck {
                vm,
                framework,
                output,
            } => {
                handlers::security::handle_compliance_check(vm, framework, output, &cli.namespace)
                    .await?;
            }
            SecurityCommands::ComplianceReport {
                vm,
                report_id,
                output,
            } => {
                handlers::security::handle_compliance_report(vm, report_id, output, &cli.namespace)
                    .await?;
            }
            SecurityCommands::AuditList {
                vm,
                event_type,
                severity,
                security_only,
                output,
            } => {
                handlers::security::handle_audit_list(
                    vm,
                    event_type,
                    severity,
                    security_only,
                    output,
                    &cli.namespace,
                )?;
            }
            SecurityCommands::AuditGet { log_id, output } => {
                handlers::security::handle_audit_get(log_id, output, &cli.namespace)?;
            }
            SecurityCommands::AuditStats { vm, period, output } => {
                handlers::security::handle_audit_stats(vm, period, output, &cli.namespace)?;
            }
        },

        Commands::Cost(cmd) => match cmd {
            CostCommands::CostAnalyze { vm, period, output } => {
                handlers::cost::handle_cost_analyze(vm, period, output, &cli.namespace).await?
            }
            CostCommands::CostSummary {
                namespace,
                period,
                group_by,
                output,
            } => handlers::cost::handle_cost_summary(namespace, period, group_by, output).await?,
            CostCommands::CostReport {
                report_type,
                format,
                output,
            } => handlers::cost::handle_cost_report(report_type, format, output)?,
            CostCommands::BudgetList { output } => handlers::cost::handle_budget_list(output)?,
            CostCommands::BudgetCreate {
                name,
                amount,
                period,
                scope,
                alert_threshold,
            } => {
                handlers::cost::handle_budget_create(name, amount, period, scope, alert_threshold)?
            }
            CostCommands::BudgetStatus { name, output } => {
                handlers::cost::handle_budget_status(name, output)?
            }
            CostCommands::CostOptimize {
                vm,
                high_priority_only,
                output,
            } => handlers::cost::handle_cost_optimize(vm, high_priority_only, output)?,
            CostCommands::CostWaste {
                waste_type,
                min_waste,
                output,
            } => handlers::cost::handle_cost_waste(waste_type, min_waste, output)?,
            CostCommands::CostForecast {
                budget,
                period,
                output,
            } => handlers::cost::handle_cost_forecast(budget, period, output)?,
        },

        Commands::Automation(cmd) => match cmd {
            AutomationCommands::AutomationList {
                enabled_only,
                output,
            } => handlers::automation::handle_automation_list(enabled_only, output)?,
            AutomationCommands::AutomationCreate {
                name,
                description,
                trigger,
                enable,
            } => {
                handlers::automation::handle_automation_create(name, description, trigger, enable)?
            }
            AutomationCommands::AutomationGet { rule, output } => {
                handlers::automation::handle_automation_get(rule, output)?
            }
            AutomationCommands::AutomationRun { rule, dry_run } => {
                handlers::automation::handle_automation_run(rule, dry_run)?
            }
            AutomationCommands::WorkflowList { output } => {
                handlers::automation::handle_workflow_list(output)?
            }
            AutomationCommands::WorkflowCreate {
                name,
                description,
                template,
            } => handlers::automation::handle_workflow_create(name, description, template)?,
            AutomationCommands::WorkflowGet { workflow, output } => {
                handlers::automation::handle_workflow_get(workflow, output)?
            }
            AutomationCommands::WorkflowRun { workflow, watch } => {
                handlers::automation::handle_workflow_run(workflow, watch)?
            }
            AutomationCommands::WorkflowExecutions {
                workflow,
                limit,
                output,
            } => handlers::automation::handle_workflow_executions(workflow, limit, output)?,
            AutomationCommands::ScheduleList {
                enabled_only,
                output,
            } => handlers::automation::handle_schedule_list(enabled_only, output)?,
            AutomationCommands::ScheduleCreate {
                name,
                rule,
                schedule,
                enable,
            } => handlers::automation::handle_schedule_create(name, rule, schedule, enable)?,
        },

        Commands::Observability(cmd) => match cmd {
            ObservabilityCommands::LogsQuery {
                start,
                end,
                level,
                source,
                search,
                limit,
            } => handlers::observability::handle_logs_query(
                start, end, level, source, search, limit,
            )?,
            ObservabilityCommands::LogsStats { group_by } => {
                handlers::observability::handle_logs_stats(group_by)?
            }
            ObservabilityCommands::LogsPatterns { min_count } => {
                handlers::observability::handle_logs_patterns(min_count)?
            }
            ObservabilityCommands::MetricsCollect { vm } => {
                handlers::observability::handle_metrics_collect(vm).await?
            }
            ObservabilityCommands::MetricsQuery {
                name,
                start,
                end,
                aggregation,
            } => handlers::observability::handle_metrics_query(name, start, end, aggregation)?,
            ObservabilityCommands::MetricsSnapshot {
                vm,
                cpu_threshold,
                memory_threshold,
            } => handlers::observability::handle_metrics_snapshot(
                vm,
                cpu_threshold,
                memory_threshold,
            )?,
            ObservabilityCommands::AlertsList {
                enabled_only,
                severity,
                output,
            } => handlers::observability::handle_alerts_list(enabled_only, severity, output)?,
            ObservabilityCommands::AlertsCreate {
                name,
                severity,
                metric,
                operator,
                threshold,
                duration,
            } => handlers::observability::handle_alerts_create(
                name, severity, metric, operator, threshold, duration,
            )?,
            ObservabilityCommands::AlertsActive { severity, output } => {
                handlers::observability::handle_alerts_active(severity, output)?
            }
            ObservabilityCommands::AlertsResolve { alert_id } => {
                handlers::observability::handle_alerts_resolve(alert_id)?
            }
            ObservabilityCommands::InsightsGenerate {
                vm,
                insight_type,
                min_severity,
            } => handlers::observability::handle_insights_generate(vm, insight_type, min_severity)?,
            ObservabilityCommands::Recommendations {
                category,
                min_priority,
                with_savings,
                output,
            } => handlers::observability::handle_recommendations(
                category,
                min_priority,
                with_savings,
                output,
            )?,
            ObservabilityCommands::TrendsAnalyze {
                metric,
                window,
                threshold,
            } => handlers::observability::handle_trends_analyze(metric, window, threshold)?,
            ObservabilityCommands::HealthCheck { component, output } => {
                handlers::observability::handle_health_check(component, output).await?
            }
            ObservabilityCommands::EventList { vm, limit, output } => {
                handlers::api::handle_event_list(cli.namespace.clone(), vm, limit, output)?
            }
            ObservabilityCommands::EventRecent { limit, output } => {
                handlers::api::handle_event_recent(cli.namespace.clone(), limit, output)?
            }
        },

        Commands::Tenancy(cmd) => match cmd {
            TenancyCommands::TenantsList {
                active_only,
                output,
            } => handlers::multitenancy::handle_tenants_list(active_only, output)?,
            TenancyCommands::TenantsCreate {
                name,
                owner,
                email,
                description,
                namespace,
            } => handlers::multitenancy::handle_tenants_create(
                name,
                owner,
                email,
                description,
                namespace,
            )?,
            TenancyCommands::TenantsShow { tenant, output } => {
                handlers::multitenancy::handle_tenants_show(tenant, output)?
            }
            TenancyCommands::TenantsDelete { tenant, yes } => {
                handlers::multitenancy::handle_tenants_delete(tenant, yes)?
            }
            TenancyCommands::UsersList {
                active_only,
                group,
                output,
            } => handlers::multitenancy::handle_users_list(active_only, group, output)?,
            TenancyCommands::UsersCreate {
                username,
                email,
                role,
                group,
            } => handlers::multitenancy::handle_users_create(username, email, role, group)?,
            TenancyCommands::UsersAssignRole { user, role, scope } => {
                handlers::multitenancy::handle_users_assign_role(user, role, scope)?
            }
            TenancyCommands::RolesList {
                builtin,
                custom,
                output,
            } => handlers::multitenancy::handle_roles_list(builtin, custom, output)?,
            TenancyCommands::RolesShow { role, output } => {
                handlers::multitenancy::handle_roles_show(role, output)?
            }
            TenancyCommands::RolesCreate {
                name,
                description,
                permissions,
            } => handlers::multitenancy::handle_roles_create(name, description, permissions)?,
            TenancyCommands::QuotasList {
                namespace,
                exceeded,
                output,
            } => handlers::multitenancy::handle_quotas_list(namespace, exceeded, output)?,
            TenancyCommands::QuotasCreate {
                name,
                namespace,
                preset,
            } => handlers::multitenancy::handle_quotas_create(name, namespace, preset).await?,
            TenancyCommands::QuotasShow {
                quota,
                utilization,
                output,
            } => handlers::multitenancy::handle_quotas_show(quota, utilization, output)?,
            TenancyCommands::GroupsList { output } => {
                handlers::multitenancy::handle_groups_list(output)?
            }
            TenancyCommands::GroupsCreate {
                name,
                description,
                role,
            } => handlers::multitenancy::handle_groups_create(name, description, role)?,
            TenancyCommands::GroupsAddUser { group, user } => {
                handlers::multitenancy::handle_groups_add_user(group, user)?
            }
        },

        Commands::Inventory(cmd) => match cmd {
            InventoryCommands::Inventory {
                all_namespaces,
                datacenter,
                cluster,
                folder,
                output,
            } => {
                handlers::vcenter::handle_inventory(
                    all_namespaces,
                    output,
                    datacenter,
                    cluster,
                    folder,
                    &cli.namespace,
                    cli.kubeconfig.as_deref(),
                )
                .await?
            }
            InventoryCommands::TagSet { vm, key, value } => {
                handlers::vcenter::handle_vm_metadata(
                    vm,
                    &cli.namespace,
                    crate::vcenter_ops::metadata::MetadataKind::Tag,
                    key,
                    Some(value),
                    cli.kubeconfig.as_deref(),
                )
                .await?
            }
            InventoryCommands::TagRemove { vm, key } => {
                handlers::vcenter::handle_vm_metadata(
                    vm,
                    &cli.namespace,
                    crate::vcenter_ops::metadata::MetadataKind::Tag,
                    key,
                    None,
                    cli.kubeconfig.as_deref(),
                )
                .await?
            }
            InventoryCommands::AttributeSet { vm, key, value } => {
                handlers::vcenter::handle_vm_metadata(
                    vm,
                    &cli.namespace,
                    crate::vcenter_ops::metadata::MetadataKind::Attribute,
                    key,
                    Some(value),
                    cli.kubeconfig.as_deref(),
                )
                .await?
            }
            InventoryCommands::AttributeRemove { vm, key } => {
                handlers::vcenter::handle_vm_metadata(
                    vm,
                    &cli.namespace,
                    crate::vcenter_ops::metadata::MetadataKind::Attribute,
                    key,
                    None,
                    cli.kubeconfig.as_deref(),
                )
                .await?
            }
            InventoryCommands::InventoryDatacenterSet { vm, datacenter } => {
                handlers::vcenter::handle_vm_metadata(
                    vm,
                    &cli.namespace,
                    crate::vcenter_ops::metadata::MetadataKind::Datacenter,
                    "datacenter".into(),
                    Some(datacenter),
                    cli.kubeconfig.as_deref(),
                )
                .await?
            }
            InventoryCommands::InventoryClusterSet { vm, cluster } => {
                handlers::vcenter::handle_vm_metadata(
                    vm,
                    &cli.namespace,
                    crate::vcenter_ops::metadata::MetadataKind::Cluster,
                    "cluster".into(),
                    Some(cluster),
                    cli.kubeconfig.as_deref(),
                )
                .await?
            }
            InventoryCommands::InventoryFolderSet { vm, folder } => {
                handlers::vcenter::handle_vm_metadata(
                    vm,
                    &cli.namespace,
                    crate::vcenter_ops::metadata::MetadataKind::Folder,
                    "folder".into(),
                    Some(folder),
                    cli.kubeconfig.as_deref(),
                )
                .await?
            }
            InventoryCommands::Activity {
                all_namespaces,
                target,
                severity,
                kind,
                limit,
                output,
            } => {
                handlers::vcenter::handle_activity(
                    handlers::vcenter::ActivityQueryArgs {
                        all_namespaces,
                        target,
                        severity,
                        kind,
                        limit,
                        output,
                    },
                    &cli.namespace,
                    cli.kubeconfig.as_deref(),
                )
                .await?
            }
        },

        Commands::Maintenance(cmd) => match cmd {
            MaintenanceCommands::MaintenancePlan {
                node,
                max_parallel,
                output,
            } => {
                handlers::vcenter::handle_maintenance_plan(
                    node,
                    max_parallel,
                    output,
                    cli.kubeconfig.as_deref(),
                )
                .await?
            }
            MaintenanceCommands::MaintenanceEnter {
                node,
                max_parallel,
                dry_run,
                force,
            } => {
                handlers::vcenter::handle_maintenance_enter(
                    node,
                    max_parallel,
                    dry_run,
                    force,
                    cli.kubeconfig.as_deref(),
                )
                .await?
            }
            MaintenanceCommands::MaintenanceStatus { node } => {
                handlers::vcenter::handle_maintenance_status(node, cli.kubeconfig.as_deref())
                    .await?
            }
            MaintenanceCommands::MaintenanceExit { node } => {
                handlers::vcenter::handle_maintenance_exit(node, cli.kubeconfig.as_deref()).await?
            }
        },

        Commands::Placement(cmd) => match cmd {
            PlacementCommands::PlacementAdvisor {
                cpu,
                memory_gib,
                required_label,
                avoid_node,
                preferred_zone,
                output,
            } => {
                handlers::vcenter::handle_placement_advisor(
                    cpu,
                    memory_gib,
                    required_label,
                    avoid_node,
                    preferred_zone,
                    output,
                    cli.kubeconfig.as_deref(),
                )
                .await?
            }
            PlacementCommands::PlacementRebalance {
                threshold,
                max_migrations,
                output,
            } => {
                handlers::vcenter::handle_placement_rebalance(
                    threshold,
                    max_migrations,
                    output,
                    cli.kubeconfig.as_deref(),
                )
                .await?
            }
        },

        Commands::Dev(cmd) => match cmd {
            DevCommands::Completions {
                shell,
                output,
                install,
            } => handlers::devexp::handle_completions(shell, output, install)?,
            DevCommands::ConfigSave {
                name,
                file,
                description,
                category,
                tags,
            } => handlers::devexp::handle_config_save(name, file, description, category, tags)?,
            DevCommands::ConfigLoad {
                name,
                output,
                format,
            } => handlers::devexp::handle_config_load(name, output, format)?,
            DevCommands::ConfigList {
                category,
                tag,
                sort_by,
                output,
            } => handlers::devexp::handle_config_list(category, tag, sort_by, output)?,
            DevCommands::ConfigDelete { name, yes } => {
                handlers::devexp::handle_config_delete(name, yes)?
            }
            DevCommands::Diff {
                source,
                target,
                show_unchanged,
                output,
            } => handlers::devexp::handle_diff(source, target, show_unchanged, output)?,
            DevCommands::TerraformScaffold { output, url } => {
                let written =
                    crate::terraform::write_scaffold(std::path::Path::new(&output), &url)?;
                println!("Wrote Terraform scaffold:");
                for f in written {
                    println!("  {f}");
                }
            }
        },

        Commands::Change(cmd) => match cmd {
            ChangeCommands::Drift {
                desired,
                actual,
                vm,
                ignore,
                include_status,
                fail_on,
                output,
            } => {
                handlers::gitops::handle_drift(
                    desired,
                    actual,
                    vm,
                    ignore,
                    include_status,
                    fail_on,
                    output,
                    &cli.namespace,
                    cli.kubeconfig.as_deref(),
                )
                .await?
            }
            ChangeCommands::Plan {
                desired,
                actual,
                vm,
                ignore,
                include_status,
                fail_on_downtime,
                fail_on_recreate,
                output,
            } => {
                handlers::gitops::handle_change_plan(
                    desired,
                    actual,
                    vm,
                    ignore,
                    include_status,
                    fail_on_downtime,
                    fail_on_recreate,
                    output,
                    &cli.namespace,
                    cli.kubeconfig.as_deref(),
                )
                .await?
            }
        },

        Commands::Guest(cmd) => match cmd {
            GuestCommands::GuestInsight { vm, output, strict } => {
                handlers::guest::handle_guest_insight(
                    vm,
                    output,
                    strict,
                    &cli.namespace,
                    cli.kubeconfig.as_deref(),
                )
                .await?
            }
            GuestCommands::ImageBundle {
                name,
                version,
                source,
                source_type,
                size,
                storage_class,
                checksum,
                output,
                format,
            } => handlers::images::handle_image_bundle(
                name,
                version,
                source,
                source_type,
                size,
                storage_class,
                checksum,
                output,
                format,
                &cli.namespace,
            )?,
            GuestCommands::WaitReady { name, timeout } => {
                let client = crate::kube::KubeClient::new().await?;
                let report = client
                    .wait_until_guest_ready(&cli.namespace, &name, timeout)
                    .await?;
                if report.cloud_init_ready {
                    println!("VM '{name}' ready: {}", report.reason);
                } else {
                    anyhow::bail!("VM '{name}' not ready after {timeout}s: {}", report.reason);
                }
            }
            GuestCommands::WaitImage { name, timeout } => {
                let client = crate::kube::KubeClient::new().await?;
                match client
                    .wait_for_data_volume(&cli.namespace, &name, timeout)
                    .await?
                {
                    crate::kube::cdi::DataVolumeWait::Ready => {
                        println!("DataVolume '{name}' succeeded");
                    }
                    crate::kube::cdi::DataVolumeWait::Failed => {
                        anyhow::bail!("DataVolume '{name}' failed");
                    }
                    crate::kube::cdi::DataVolumeWait::Pending => {
                        anyhow::bail!("DataVolume '{name}' still pending after {timeout}s");
                    }
                }
            }
        },

        Commands::Api(cmd) => match cmd {
            ApiCommands::ApiServe {
                port,
                host,
                tls,
                tls_cert,
                tls_key,
                auth,
                rate_limit,
            } => {
                let port = port.unwrap_or(app_config.api.port);
                let host = host.unwrap_or_else(|| app_config.api.host.clone());
                let tls = tls || app_config.api.tls;
                let tls_cert = tls_cert.or(app_config.api.tls_cert.clone());
                let tls_key = tls_key.or(app_config.api.tls_key.clone());
                let auth = auth.unwrap_or_else(|| app_config.api.auth.clone());
                let rate_limit = rate_limit.unwrap_or(app_config.api.rate_limit);
                handlers::api::handle_api_serve(
                    port,
                    host,
                    cli.namespace.clone(),
                    tls,
                    tls_cert,
                    tls_key,
                    auth,
                    rate_limit,
                )
                .await?;
            }
            ApiCommands::ApiStatus { output } => handlers::api::handle_api_status(output)?,
            ApiCommands::ApiRoutes { method, output } => {
                handlers::api::handle_api_routes(method, output)?
            }
            ApiCommands::ApiSpec { format, output } => {
                handlers::api::handle_api_spec(format, output)?
            }
            ApiCommands::ApiKeyList {
                active_only,
                output,
            } => handlers::api::handle_api_key_list(active_only, output)?,
            ApiCommands::ApiKeyCreate {
                name,
                permissions,
                rate_limit,
            } => handlers::api::handle_api_key_create(name, permissions, rate_limit)?,
            ApiCommands::ApiKeyDelete { key, yes } => {
                handlers::api::handle_api_key_delete(key, yes)?
            }
            ApiCommands::WebhookList {
                active_only,
                output,
            } => handlers::api::handle_webhook_list(active_only, output)?,
            ApiCommands::WebhookCreate {
                name,
                url,
                events,
                secret,
            } => handlers::api::handle_webhook_create(name, url, events, secret)?,
            ApiCommands::WebhookDelete { webhook, yes } => {
                handlers::api::handle_webhook_delete(webhook, yes)?
            }
            ApiCommands::Tui {
                no_splash,
                theme,
                interactive,
            } => {
                handlers::api::handle_tui(cli.namespace.clone(), theme, interactive, no_splash)
                    .await?
            }
        },

        Commands::Config(cmd) => match cmd {
            ConfigCommands::ConfigShow { path } => {
                let user_path = AppConfig::user_path()?;
                let system_path = AppConfig::system_path();
                if path {
                    println!("{}", user_path.display());
                } else {
                    use tui::colors::cli as color;
                    println!("{}", color::header("Zorvia Configuration"));
                    println!();
                    println!(
                        "  System config: {}  {}",
                        system_path.display(),
                        if system_path.exists() {
                            color::success("(loaded)")
                        } else {
                            color::muted("(not found)")
                        }
                    );
                    println!(
                        "  User config:   {}  {}",
                        user_path.display(),
                        if user_path.exists() {
                            color::success("(loaded)")
                        } else {
                            color::muted("(not found)")
                        }
                    );
                    println!();
                    println!("  Priority: CLI args > user config > system config > defaults");
                    println!();

                    let content = toml::to_string_pretty(&app_config)?;
                    println!("{}", content);
                }
            }
            ConfigCommands::ConfigInit { force } => {
                use tui::colors::cli as color;
                let config_path = AppConfig::default_path()?;

                if config_path.exists() && !force {
                    println!(
                        "{}",
                        color::warning(&format!(
                            "Config file already exists: {}",
                            config_path.display()
                        ))
                    );
                    println!("  Use --force to overwrite");
                    return Ok(());
                }

                app_config.save()?;
                println!(
                    "{}",
                    color::success(&format!("✓ Config file created: {}", config_path.display()))
                );
                println!();
                println!("Edit it to customize defaults:");
                println!("  namespace, API port/host, logging level, output format, etc.");
            }
        },

        Commands::Init {
            name,
            project_type,
            directory,
            namespace,
            no_examples,
            ci,
            no_git,
        } => handlers::devexp::handle_init(
            name,
            project_type,
            directory,
            namespace,
            no_examples,
            ci,
            no_git,
        )?,
        Commands::Info {
            detailed,
            diagnostics,
            output,
        } => handlers::devexp::handle_info(detailed, diagnostics, output, &cli.namespace)?,
        Commands::Adopt {
            all_namespaces,
            output,
        } => handlers::adopt::handle_adopt(all_namespaces, output, &cli.namespace).await?,
        Commands::CommandList => {
            use tui::colors::cli as color;
            println!("{}", color::header("Zorvia Commands"));
            println!();
            let groups = [
                (
                    "VM lifecycle: create, list, inspect, and control virtual machines",
                    "vm",
                    vec![
                        "create",
                        "list",
                        "get",
                        "delete",
                        "start",
                        "stop",
                        "restart",
                        "pause",
                        "resume",
                        "console",
                        "ssh",
                        "vnc",
                        "logs",
                        "status",
                        "clone",
                        "resources",
                        "export",
                        "wizard",
                        "batch",
                    ],
                ),
                (
                    "OS templates and config generation",
                    "template",
                    vec!["generate", "templates", "images", "show", "validate"],
                ),
                (
                    "Reusable VM configuration profiles",
                    "profile",
                    vec![
                        "profiles",
                        "show",
                        "profile-create",
                        "profile-edit",
                        "profile-delete",
                    ],
                ),
                (
                    "Multi-VM blueprints and deployment",
                    "blueprint",
                    vec![
                        "blueprints",
                        "show",
                        "deploy",
                        "blueprint-create",
                        "blueprint-edit",
                        "blueprint-delete",
                        "blueprint-validate",
                    ],
                ),
                (
                    "Health checks and recommendations",
                    "advisor",
                    vec!["health", "recommend"],
                ),
                (
                    "VM disk snapshots",
                    "snapshot",
                    vec![
                        "snapshot-create",
                        "snapshot-list",
                        "snapshot-get",
                        "snapshot-delete",
                        "snapshot-restore",
                    ],
                ),
                (
                    "Live resource monitoring",
                    "monitor",
                    vec![
                        "monitor-live",
                        "monitor-stats",
                        "monitor-compare",
                        "monitor-top",
                    ],
                ),
                (
                    "Disk management",
                    "disk",
                    vec!["disk-expand", "disk-health", "disk-script", "disk-usage"],
                ),
                (
                    "Networking",
                    "network",
                    vec![
                        "network-list",
                        "network-get",
                        "network-bandwidth",
                        "network-traffic",
                        "network-policies",
                        "network-policy",
                    ],
                ),
                (
                    "VM live migration",
                    "migration",
                    vec!["migrate", "migration-status", "migration-list"],
                ),
                (
                    "High availability and node evacuation",
                    "ha",
                    vec![
                        "ha-config",
                        "ha-status",
                        "evacuate-node",
                        "evacuation-status",
                    ],
                ),
                (
                    "Backup, restore, and disaster recovery",
                    "backup",
                    vec![
                        "backup-create",
                        "backup-list",
                        "backup-get",
                        "backup-delete",
                        "backup-restore",
                        "backup-verify",
                        "backup-schedules",
                        "backup-schedule-create",
                        "recovery-plan",
                        "recovery-execute",
                    ],
                ),
                (
                    "Security scanning, compliance, and audit",
                    "security",
                    vec![
                        "security-scan",
                        "security-assess",
                        "security-harden",
                        "security-profiles",
                        "compliance-check",
                        "compliance-report",
                        "audit-list",
                        "audit-get",
                        "audit-stats",
                    ],
                ),
                (
                    "Cost analysis, budgets, and optimization",
                    "cost",
                    vec![
                        "cost-analyze",
                        "cost-summary",
                        "cost-report",
                        "budget-list",
                        "budget-create",
                        "budget-status",
                        "cost-optimize",
                        "cost-waste",
                        "cost-forecast",
                    ],
                ),
                (
                    "Automation, workflows, and schedules",
                    "automation",
                    vec![
                        "automation-list",
                        "automation-create",
                        "automation-get",
                        "automation-run",
                        "workflow-list",
                        "workflow-create",
                        "workflow-get",
                        "workflow-run",
                        "workflow-executions",
                        "schedule-list",
                        "schedule-create",
                    ],
                ),
                (
                    "Logs, metrics, alerts, and insights",
                    "observability",
                    vec![
                        "logs-query",
                        "logs-stats",
                        "logs-patterns",
                        "metrics-collect",
                        "metrics-query",
                        "metrics-snapshot",
                        "alerts-list",
                        "alerts-create",
                        "alerts-active",
                        "alerts-resolve",
                        "insights-generate",
                        "recommendations",
                        "trends-analyze",
                        "health-check",
                        "event-list",
                        "event-recent",
                    ],
                ),
                (
                    "Multi-tenancy: tenants, users, roles, quotas, groups",
                    "tenancy",
                    vec![
                        "tenants-list",
                        "tenants-create",
                        "tenants-show",
                        "tenants-delete",
                        "users-list",
                        "users-create",
                        "users-assign-role",
                        "roles-list",
                        "roles-show",
                        "roles-create",
                        "quotas-list",
                        "quotas-create",
                        "quotas-show",
                        "groups-list",
                        "groups-create",
                        "groups-add-user",
                    ],
                ),
                (
                    "Inventory, tags, attributes, and activity",
                    "inventory",
                    vec![
                        "show",
                        "tag-set",
                        "tag-remove",
                        "attribute-set",
                        "attribute-remove",
                        "inventory-datacenter-set",
                        "inventory-cluster-set",
                        "inventory-folder-set",
                        "activity",
                    ],
                ),
                (
                    "Maintenance mode",
                    "maintenance",
                    vec![
                        "maintenance-plan",
                        "maintenance-enter",
                        "maintenance-status",
                        "maintenance-exit",
                    ],
                ),
                (
                    "Placement advice and rebalancing",
                    "placement",
                    vec!["placement-advisor", "placement-rebalance"],
                ),
                (
                    "Developer tooling",
                    "dev",
                    vec![
                        "completions",
                        "config-save",
                        "config-load",
                        "config-list",
                        "config-delete",
                        "diff",
                        "terraform-scaffold",
                    ],
                ),
                (
                    "Drift detection and change planning",
                    "change",
                    vec!["drift", "plan"],
                ),
                (
                    "Guest agent insight and image readiness",
                    "guest",
                    vec!["guest-insight", "image-bundle", "wait-ready", "wait-image"],
                ),
                (
                    "API server, keys, webhooks, and TUI",
                    "api",
                    vec![
                        "api-serve",
                        "api-status",
                        "api-routes",
                        "api-spec",
                        "api-key-list",
                        "api-key-create",
                        "api-key-delete",
                        "webhook-list",
                        "webhook-create",
                        "webhook-delete",
                        "tui",
                    ],
                ),
                (
                    "Zorvia's own CLI configuration file",
                    "config",
                    vec!["config-show", "config-init"],
                ),
            ];

            for (group_doc, group_key, cmds) in &groups {
                println!("  {}", color::header(group_doc));
                for cmd in cmds {
                    println!("    {}", color::value(&format!("{} {}", group_key, cmd)));
                }
                println!();
            }

            println!("  {}", color::header("Top-level"));
            for cmd in ["init", "info", "adopt", "commands"] {
                println!("    {}", color::value(cmd));
            }
            println!();

            println!(
                "{}",
                color::muted(
                    "Use 'zorvia <group> <command> --help' for details on a specific command"
                )
            );
        }
    }

    Ok(())
}
