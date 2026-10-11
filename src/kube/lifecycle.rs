//! Production lifecycle helpers that do not require a live cluster to test.
//!
//! Path builders and validation for KubeVirt VMI subresources used by
//! pause/resume, serial console, and VNC.

use anyhow::{bail, Result};
use regex::Regex;
use std::sync::OnceLock;

/// RFC 1123 DNS label used by Kubernetes object names.
fn k8s_name_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[a-z0-9]([-a-z0-9]*[a-z0-9])?$").expect("valid regex"))
}

/// Validate a Kubernetes namespace or object name.
pub fn validate_k8s_name(kind: &str, name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 63 {
        bail!("{kind} must be 1-63 characters");
    }
    if !k8s_name_re().is_match(name) {
        bail!("{kind} '{name}' is not a valid RFC 1123 DNS label");
    }
    Ok(())
}

/// Allowed VMI subresources Zorvia proxies or invokes.
pub const VMI_SUBRESOURCES: &[&str] = &[
    "console",
    "vnc",
    "pause",
    "unpause",
    "freeze",
    "unfreeze",
    "guestosinfo",
    "userlist",
    "filesystemlist",
    "addvolume",
    "removevolume",
    "addinterface",
    "removeinterface",
];

/// WebSocket subprotocol expected by KubeVirt for a given console type.
pub fn kubevirt_ws_subprotocol(subresource: &str) -> Option<&'static str> {
    match subresource {
        "vnc" => Some("binary.kubevirt.io"),
        "console" => Some("plain.kubevirt.io"),
        _ => None,
    }
}

/// Build the apiserver path for a VMI subresource.
pub fn vmi_subresource_path(namespace: &str, name: &str, subresource: &str) -> Result<String> {
    validate_k8s_name("namespace", namespace)?;
    validate_k8s_name("name", name)?;
    if !VMI_SUBRESOURCES.contains(&subresource) {
        bail!("unsupported VMI subresource '{subresource}'");
    }
    Ok(format!(
        "/apis/subresources.kubevirt.io/v1/namespaces/{namespace}/virtualmachineinstances/{name}/{subresource}"
    ))
}

/// Authenticated WebSocket path served by Zorvia for serial or VNC.
pub fn zorvia_console_ws_path(kind: &str, vm: &str) -> Result<String> {
    validate_k8s_name("name", vm)?;
    match kind {
        "console" | "vnc" => Ok(format!("/ws/{kind}/{vm}")),
        other => bail!("unsupported console kind '{other}'"),
    }
}

/// True when KubeVirt printable status / phase indicates a paused guest.
pub fn status_is_paused(
    printable: Option<&str>,
    phase: Option<&str>,
    conditions: &[(&str, &str)],
) -> bool {
    if printable
        .map(|s| s.eq_ignore_ascii_case("paused") || s.to_ascii_lowercase().contains("paus"))
        .unwrap_or(false)
    {
        return true;
    }
    if phase
        .map(|s| s.eq_ignore_ascii_case("paused"))
        .unwrap_or(false)
    {
        return true;
    }
    conditions.iter().any(|(ty, status)| {
        ty.eq_ignore_ascii_case("Paused") && status.eq_ignore_ascii_case("True")
    })
}

/// Map a KubeVirt / HTTP error into a client-safe pause/resume message.
pub fn classify_lifecycle_error(action: &str, raw: &str) -> (u16, &'static str, String) {
    let lower = raw.to_ascii_lowercase();
    if lower.contains("notfound") || lower.contains("not found") {
        return (
            404,
            "NOT_FOUND",
            format!("VM instance not found; start the VM before {action}"),
        );
    }
    if lower.contains("forbidden") || lower.contains("unauthorized") {
        return (
            403,
            "FORBIDDEN",
            format!("Not allowed to {action} this VM (check subresources.kubevirt.io RBAC)"),
        );
    }
    if lower.contains("already paused") {
        return (
            409,
            "CONFLICT",
            format!("Cannot {action}: VM is already paused"),
        );
    }
    if lower.contains("not paused") {
        return (
            409,
            "CONFLICT",
            format!("Cannot {action}: VM is not paused"),
        );
    }
    if lower.contains("conflict") {
        // Deliberately templated rather than echoing `raw`: kube-rs's ApiError
        // Display includes the full Rust Debug output of the K8s ErrorResponse
        // struct, which is noisy/unprofessional to surface verbatim in a JSON
        // API response.
        return (
            409,
            "CONFLICT",
            format!("Cannot {action}: VM is in a conflicting state, try again"),
        );
    }
    (500, "LIFECYCLE_FAILED", format!("Failed to {action} VM"))
}

/// Map a KubeVirt / HTTP error into a client-safe migration message.
pub fn classify_migration_error(action: &str, raw: &str) -> (u16, &'static str, String) {
    let lower = raw.to_ascii_lowercase();
    if lower.contains("already exists") || lower.contains("exists") {
        return (
            409,
            "CONFLICT",
            format!("Cannot {action}: a migration with this name already exists"),
        );
    }
    if lower.contains("notfound") || lower.contains("not found") || lower.contains("does not exist")
    {
        // KubeVirt's migration-create-validator admission webhook rejects a
        // migration for a nonexistent VMI with "...the VMI ... does not
        // exist" — a 200-level apiserver response, not a 404, so it needs
        // its own check alongside the generic NotFound patterns above.
        return (
            404,
            "NOT_FOUND",
            format!("Cannot {action}: VM instance not found; start the VM before migrating"),
        );
    }
    if lower.contains("forbidden") || lower.contains("unauthorized") {
        return (
            403,
            "FORBIDDEN",
            format!("Not allowed to {action} (check virtualmachineinstancemigrations RBAC)"),
        );
    }
    if lower.contains("admission webhook") && lower.contains("denied the request") {
        return (
            400,
            "VALIDATION_FAILED",
            format!("Cannot {action}: rejected by KubeVirt (VM may not be in a migratable state)"),
        );
    }
    (500, "MIGRATION_FAILED", format!("Failed to {action}"))
}

/// Map a KubeVirt / HTTP error into a client-safe hotplug message.
pub fn classify_hotplug_error(action: &str, raw: &str) -> (u16, &'static str, String) {
    let lower = raw.to_ascii_lowercase();
    if lower.contains("hotplug_not_provisioned") {
        return (
            409,
            "HOTPLUG_NOT_PROVISIONED",
            format!("Cannot {action}: this VM was not created with hotplug headroom (maxSockets/maxGuest); recreate it to enable live resize"),
        );
    }
    if lower.contains("hotplug_limit_exceeded") {
        return (
            409,
            "HOTPLUG_LIMIT_EXCEEDED",
            format!("Cannot {action}: {raw}"),
        );
    }
    if lower.contains("volume_not_found") {
        return (404, "VOLUME_NOT_FOUND", format!("Cannot {action}: {raw}"));
    }
    if lower.contains("notfound") || lower.contains("not found") {
        // A bare "404: Page Not Found" (no structured k8s ApiError with a
        // named resource) from an addinterface/removeinterface call almost
        // always means the subresource route itself doesn't exist on this
        // KubeVirt version — not that the running VM instance is missing.
        // Reporting "VM instance not found; start the VM" for that case is
        // actively misleading when the VM is demonstrably running.
        let is_nic_action = action.to_ascii_lowercase().contains("nic");
        let looks_like_missing_route =
            !lower.contains("apierror") && lower.contains("page not found");
        if is_nic_action && looks_like_missing_route {
            return (
                501,
                "UNSUPPORTED",
                format!("{action} is not supported on this cluster (requires the KubeVirt HotplugNICs feature gate + Multus)"),
            );
        }
        return (
            404,
            "NOT_FOUND",
            format!("VM instance not found; start the VM before {action}"),
        );
    }
    if lower.contains("forbidden") || lower.contains("unauthorized") {
        return (
            403,
            "FORBIDDEN",
            format!("Not allowed to {action} this VM (check subresources.kubevirt.io RBAC)"),
        );
    }
    if lower.contains("methodnotallowed")
        || lower.contains("method not allowed")
        || (lower.contains("interface")
            && (lower.contains("not supported") || lower.contains("unsupported")))
    {
        return (
            501,
            "UNSUPPORTED",
            format!("{action} is not supported on this cluster (requires the KubeVirt HotplugNICs feature gate + Multus)"),
        );
    }
    if lower.contains("conflict") || lower.contains("already exists") {
        return (409, "CONFLICT", format!("Cannot {action}: {raw}"));
    }
    (500, "HOTPLUG_FAILED", format!("Failed to {action}"))
}

/// Map a KubeVirt / HTTP error into a client-safe snapshot-restore message.
/// Like the other `classify_*_error` functions, this must receive *raw*
/// error text (before `sanitize_error()`), since it's the thing doing the
/// sanitizing here -- its return value is always client-safe, never the raw
/// input, so callers should use the returned message as-is instead of also
/// running it through `sanitize_error()`.
pub fn classify_restore_error(action: &str, raw: &str) -> (u16, &'static str, String) {
    let lower = raw.to_ascii_lowercase();
    if lower.contains("notfound") || lower.contains("not found") || lower.contains("does not exist")
    {
        return (
            404,
            "NOT_FOUND",
            format!("Cannot {action}: the snapshot or VM was not found"),
        );
    }
    if lower.contains("forbidden") || lower.contains("unauthorized") {
        return (
            403,
            "FORBIDDEN",
            format!("Not allowed to {action} (check virtualmachinerestores RBAC)"),
        );
    }
    if (lower.contains("admission webhook") && lower.contains("denied the request"))
        || lower.contains("no volumes")
        || lower.contains("excluded")
        || lower.contains("nothing to restore")
    {
        return (
            409,
            "VALIDATION_FAILED",
            format!(
                "Cannot {action}: this snapshot has no restorable disk content (its volumes \
                 were excluded at snapshot time -- this happens for VMs without \
                 PVC/DataVolume-backed disks, e.g. ephemeral emptyDisk volumes)"
            ),
        );
    }
    if lower.contains("currently running") || lower.contains("stop the vm") {
        return (
            409,
            "VM_RUNNING",
            format!("Cannot {action}: the VM is running. Stop it first and retry"),
        );
    }
    if lower.contains("conflict") || lower.contains("already exists") {
        return (
            409,
            "CONFLICT",
            format!("Cannot {action}: a restore is already in progress or in a conflicting state"),
        );
    }
    (500, "REVERT_FAILED", format!("Failed to {action}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_vm_restore_is_a_409_with_guidance() {
        let (code, kind, msg) = classify_restore_error(
            "revert to this snapshot",
            "Cannot restore in-place: VM 'x' is currently running. Stop the VM first.",
        );
        assert_eq!((code, kind), (409, "VM_RUNNING"));
        assert!(msg.contains("Stop it first"));
    }

    #[test]
    fn rejects_empty_and_invalid_names() {
        assert!(validate_k8s_name("name", "").is_err());
        assert!(validate_k8s_name("name", "VM_1").is_err());
        assert!(validate_k8s_name("name", "has.dot").is_err());
        assert!(validate_k8s_name("name", "prod-db").is_ok());
        assert!(validate_k8s_name("name", "a").is_ok());
    }

    #[test]
    fn builds_pause_unpause_paths() {
        let pause = vmi_subresource_path("default", "prod-db", "pause").unwrap();
        assert_eq!(
            pause,
            "/apis/subresources.kubevirt.io/v1/namespaces/default/virtualmachineinstances/prod-db/pause"
        );
        let unpause = vmi_subresource_path("kubevirt", "web-01", "unpause").unwrap();
        assert!(unpause.ends_with("/web-01/unpause"));
    }

    #[test]
    fn rejects_unknown_subresource() {
        assert!(vmi_subresource_path("default", "vm", "explode").is_err());
    }

    #[test]
    fn vnc_and_console_protocols() {
        assert_eq!(kubevirt_ws_subprotocol("vnc"), Some("binary.kubevirt.io"));
        assert_eq!(
            kubevirt_ws_subprotocol("console"),
            Some("plain.kubevirt.io")
        );
        assert_eq!(kubevirt_ws_subprotocol("pause"), None);
        assert_eq!(
            zorvia_console_ws_path("vnc", "win-01").unwrap(),
            "/ws/vnc/win-01"
        );
        assert_eq!(
            zorvia_console_ws_path("console", "linux-01").unwrap(),
            "/ws/console/linux-01"
        );
        assert!(zorvia_console_ws_path("spice", "vm").is_err());
    }

    #[test]
    fn paused_detection() {
        assert!(status_is_paused(Some("Paused"), None, &[]));
        assert!(status_is_paused(None, Some("Paused"), &[]));
        assert!(status_is_paused(
            None,
            Some("Running"),
            &[("Paused", "True")]
        ));
        assert!(!status_is_paused(
            Some("Running"),
            Some("Running"),
            &[("Ready", "True")]
        ));
    }

    #[test]
    fn classifies_not_found() {
        let (code, kind, _) = classify_lifecycle_error("pause", "ApiError: NotFound");
        assert_eq!(code, 404);
        assert_eq!(kind, "NOT_FOUND");
    }

    #[test]
    fn classifies_already_paused_conflict_without_echoing_raw_debug_output() {
        // Verified against a real cluster: kube-rs's ApiError Display embeds
        // the full Rust Debug output of the K8s ErrorResponse struct
        // (`ErrorResponse { status: ..., message: ..., reason: ..., code: ... }`)
        // — the response message must never echo that verbatim.
        let raw = "ApiError: Operation cannot be fulfilled on virtualmachineinstance.kubevirt.io \"demo-linux\": VMI is already paused: Conflict (ErrorResponse { status: \"Failure\", message: \"...\", reason: \"Conflict\", code: 409 })";
        let (code, kind, msg) = classify_lifecycle_error("pause", raw);
        assert_eq!(code, 409);
        assert_eq!(kind, "CONFLICT");
        assert!(!msg.contains("ErrorResponse"));
        assert!(!msg.contains("Failure"));
    }

    #[test]
    fn classifies_zorvia_vm_not_found_display_text() {
        // Regression: handlers must pass classify_lifecycle_error the RAW
        // error text, not output already collapsed by sanitize_error() —
        // ZorviaError::VmNotFound displays as "VM 'x' not found", which
        // doesn't start with any of sanitize_error's allowed prefixes and
        // used to get collapsed to a generic 500 before classification ran.
        let (code, kind, _) = classify_lifecycle_error("pause", "VM 'demo-linux' not found");
        assert_eq!(code, 404);
        assert_eq!(kind, "NOT_FOUND");
    }

    #[test]
    fn builds_addvolume_and_addinterface_paths() {
        let addvolume = vmi_subresource_path("default", "prod-db", "addvolume").unwrap();
        assert!(addvolume.ends_with("/prod-db/addvolume"));
        let addinterface = vmi_subresource_path("default", "prod-db", "addinterface").unwrap();
        assert!(addinterface.ends_with("/prod-db/addinterface"));
        let removeinterface =
            vmi_subresource_path("default", "prod-db", "removeinterface").unwrap();
        assert!(removeinterface.ends_with("/prod-db/removeinterface"));
    }

    #[test]
    fn classifies_hotplug_not_provisioned() {
        let (code, kind, _) = classify_hotplug_error(
            "hotplug CPU",
            "HOTPLUG_NOT_PROVISIONED: VM 'x' was not created with cpu.maxSockets",
        );
        assert_eq!(code, 409);
        assert_eq!(kind, "HOTPLUG_NOT_PROVISIONED");
    }

    #[test]
    fn classifies_hotplug_limit_exceeded() {
        let (code, kind, _) = classify_hotplug_error(
            "hotplug memory",
            "HOTPLUG_LIMIT_EXCEEDED: requested 8Gi exceeds maxGuest=4Gi",
        );
        assert_eq!(code, 409);
        assert_eq!(kind, "HOTPLUG_LIMIT_EXCEEDED");
    }

    #[test]
    fn classifies_volume_not_found() {
        let (code, kind, _) =
            classify_hotplug_error("hotplug disk", "VOLUME_NOT_FOUND: PVC 'x' does not exist");
        assert_eq!(code, 404);
        assert_eq!(kind, "VOLUME_NOT_FOUND");
    }

    #[test]
    fn classifies_nic_hotplug_unsupported() {
        let (code, kind, _) =
            classify_hotplug_error("hotplug NIC", "the server could not find the requested resource for interface hotplug (not supported)");
        assert_eq!(code, 501);
        assert_eq!(kind, "UNSUPPORTED");
    }

    #[test]
    fn classifies_bare_404_from_missing_nic_subresource_as_unsupported() {
        // Regression: verified live against a real cluster — when the
        // addinterface/removeinterface subresource route isn't registered
        // (this KubeVirt version's Multus/NIC-hotplug support), kube-rs
        // surfaces a bare "404: Page Not Found" with no structured
        // ApiError/resource detail. This used to be misclassified as
        // "VM instance not found; start the VM" even though the VM was
        // demonstrably running.
        let (code, kind, msg) = classify_hotplug_error("hotplug NIC", "404: Page Not Found");
        assert_eq!(code, 501);
        assert_eq!(kind, "UNSUPPORTED");
        assert!(!msg.to_ascii_lowercase().contains("start the vm"));
    }

    #[test]
    fn still_classifies_real_vm_not_found_for_non_nic_actions() {
        let (code, kind, _) = classify_hotplug_error("hotplug CPU", "404: Page Not Found");
        assert_eq!(code, 404);
        assert_eq!(kind, "NOT_FOUND");
    }

    #[test]
    fn classifies_migration_not_found() {
        let (code, kind, _) =
            classify_migration_error("migrate", "ApiError: NotFound (reason: NotFound)");
        assert_eq!(code, 404);
        assert_eq!(kind, "NOT_FOUND");
    }

    #[test]
    fn classifies_migration_admission_webhook_vmi_missing() {
        // Verified against a real cluster: KubeVirt's migration-create-validator
        // rejects a migration targeting a nonexistent VMI with this exact text.
        let (code, kind, _) = classify_migration_error(
            "migrate",
            r#"ApiError: admission webhook "migration-create-validator.kubevirt.io" denied the request: the VMI "default/x" does not exist"#,
        );
        assert_eq!(code, 404);
        assert_eq!(kind, "NOT_FOUND");
    }

    #[test]
    fn classifies_migration_other_admission_webhook_rejection() {
        let (code, kind, _) = classify_migration_error(
            "migrate",
            r#"ApiError: admission webhook "migration-create-validator.kubevirt.io" denied the request: VMI is not migratable"#,
        );
        assert_eq!(code, 400);
        assert_eq!(kind, "VALIDATION_FAILED");
    }

    #[test]
    fn classifies_migration_conflict() {
        let (code, kind, _) =
            classify_migration_error("migrate", "ApiError: AlreadyExists (reason: AlreadyExists)");
        assert_eq!(code, 409);
        assert_eq!(kind, "CONFLICT");
    }

    #[test]
    fn migration_error_message_never_echoes_raw_text() {
        let (_, _, msg) =
            classify_migration_error("migrate", "some internal detail that should not leak");
        assert!(!msg.contains("internal detail"));
    }
}
