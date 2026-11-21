// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Keeps the `secrets` RBAC rule (rescue reset-password's per-Job Secret)
//! identical across every manifest that grants it. Text-based because the Helm
//! template is not valid YAML before rendering.

use std::fs;

/// Returns the `verbs:` line (whitespace-normalised) of every rule whose
/// `resources:` is exactly `["secrets"]`.
fn secrets_verbs(path: &str) -> Vec<String> {
    let text = fs::read_to_string(format!("{}/{path}", env!("CARGO_MANIFEST_DIR")))
        .unwrap_or_else(|e| panic!("{path}: {e}"));
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let mut out = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        if l.replace(' ', "") == r#"resources:["secrets"]"# {
            let verbs = lines
                .get(i + 1)
                .filter(|v| v.starts_with("verbs:"))
                .unwrap_or_else(|| panic!("{path}: no verbs line after secrets rule"));
            out.push(verbs.replace(' ', ""));
        }
    }
    out
}

#[test]
fn secrets_rule_is_identical_in_all_manifests() {
    let want = r#"verbs:["create","delete","patch"]"#;
    // The chart has a Role-style and a ClusterRole-style block, each with it.
    for (path, count) in [
        ("charts/zorvia/templates/rbac.yaml", 2),
        ("deploy/k8s.yaml", 1),
        ("deploy/k3s-zorvia-web.yaml", 1),
    ] {
        let found = secrets_verbs(path);
        assert_eq!(
            found.len(),
            count,
            "{path}: expected {count} secrets rule(s)"
        );
        for v in found {
            assert_eq!(v, want, "{path}: secrets verbs drifted");
        }
    }
}
