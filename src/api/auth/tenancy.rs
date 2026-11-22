// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Per-user namespace allow-lists.
//!
//! A user with `namespaces = NULL` is unrestricted (the legacy behaviour and
//! what every Admin has). A user with a list is confined to those namespaces
//! and, because most routes act on the server's default namespace or on
//! cluster-wide data, to an explicit allow-list of VM-centric route prefixes.
//! Unknown routes fail closed for restricted users.

/// Route prefixes (under `/api`) a namespace-restricted identity may use.
const RESTRICTED_ALLOWED_PREFIXES: &[&str] = &[
    "/vms",
    "/v1/vms",
    "/v1/snapshots",
    "/snapshots",
    "/images",
    "/templates",
    "/v1/auth",
    "/health",
    "/v1/health",
    "/readyz",
    "/features",
    "/v1/features",
    "/instance",
    "/v1/instance",
    "/capabilities",
];

fn has_prefix(path: &str, prefix: &str) -> bool {
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Namespace named by the request itself, if any: `/v1/vms/{ns}/…`,
/// `/v1/snapshots/{ns}/…`, or a `namespace` query parameter.
fn requested_namespace(path: &str, query: Option<&str>) -> Option<String> {
    let segs: Vec<&str> = path.trim_matches('/').split('/').collect();
    if segs.len() >= 3 && segs[0] == "v1" && (segs[1] == "vms" || segs[1] == "snapshots") {
        return Some(segs[2].to_string());
    }
    query.and_then(|q| {
        q.split('&')
            .filter_map(|kv| kv.split_once('='))
            .find(|(k, _)| *k == "namespace" || *k == "ns")
            .map(|(_, v)| v.to_string())
    })
}

fn wants_all_namespaces(query: Option<&str>) -> bool {
    query.is_some_and(|q| {
        q.split('&')
            .filter_map(|kv| kv.split_once('='))
            .any(|(k, v)| k == "all_namespaces" && (v == "true" || v == "1"))
    })
}

/// `Ok(())` if a restricted identity may make this request.
pub fn check_namespace_access(
    allowed: &[String],
    default_namespace: &str,
    api_path: &str,
    query: Option<&str>,
) -> Result<(), String> {
    let path = api_path.trim_end_matches('/');
    if !RESTRICTED_ALLOWED_PREFIXES
        .iter()
        .any(|p| has_prefix(path, p))
    {
        return Err("This route is not available to namespace-restricted users".into());
    }
    if wants_all_namespaces(query) {
        return Err("all_namespaces is not available to namespace-restricted users".into());
    }
    let ns = requested_namespace(path, query).unwrap_or_else(|| default_namespace.to_string());
    if allowed.contains(&ns) {
        Ok(())
    } else {
        Err(format!("No access to namespace '{ns}'"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn explicit_namespace_must_be_allowed() {
        let al = a(&["team-a"]);
        assert!(check_namespace_access(&al, "default", "/v1/vms/team-a/web", None).is_ok());
        assert!(check_namespace_access(&al, "default", "/v1/vms/team-b/web", None).is_err());
        assert!(check_namespace_access(&al, "default", "/v1/snapshots/team-b/web", None).is_err());
        assert!(
            check_namespace_access(&al, "default", "/v1/vms", Some("namespace=team-a")).is_ok()
        );
        assert!(
            check_namespace_access(&al, "default", "/v1/vms", Some("namespace=team-b")).is_err()
        );
        assert!(check_namespace_access(&al, "default", "/v1/vms", Some("ns=kube-system")).is_err());
    }

    #[test]
    fn default_namespace_routes_need_it_allowed() {
        assert!(
            check_namespace_access(&a(&["default"]), "default", "/vms/web/start", None).is_ok()
        );
        assert!(
            check_namespace_access(&a(&["team-a"]), "default", "/vms/web/start", None).is_err()
        );
        assert!(check_namespace_access(&a(&["team-a"]), "default", "/v1/vms", None).is_err());
    }

    #[test]
    fn all_namespaces_and_unknown_routes_fail_closed() {
        let al = a(&["default"]);
        assert!(check_namespace_access(
            &al,
            "default",
            "/v1/adopt/report",
            Some("all_namespaces=true")
        )
        .is_err());
        assert!(
            check_namespace_access(&al, "default", "/v1/vms", Some("all_namespaces=true")).is_err()
        );
        for p in [
            "/audit/logs",
            "/events/stream",
            "/storage/volumes",
            "/v1/atlas/status",
            "/dashboard/overview",
            "/vmsx",
        ] {
            assert!(
                check_namespace_access(&al, "default", p, None).is_err(),
                "{p}"
            );
        }
    }
}
