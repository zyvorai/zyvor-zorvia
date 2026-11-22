// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! OIDC group -> Zorvia role (and namespace) mapping.
//!
//! Without `ZORVIA_OIDC_GROUP_ROLES` nothing changes: every OIDC user is created
//! as a plain `User` and an admin edits roles by hand. With it, the identity
//! provider decides: at **every** login the groups claim of the ID token is mapped
//! to a role (the highest matching one) and an optional namespace allow-list, so a
//! group change or removal in the IdP takes effect at the next sign-in, and a user
//! who no longer matches anything loses access (their sessions are revoked).
//!
//! `ZORVIA_OIDC_GROUP_ROLES` is a JSON array:
//! `[{"group":"zorvia-admins","role":"admin"},
//!   {"group":"team-a-devs","role":"user","namespaces":["team-a"]}]`
//! `ZORVIA_OIDC_GROUPS_CLAIM` names the claim (default `groups`);
//! `ZORVIA_OIDC_DEFAULT_ROLE` (`viewer` | `user` | `admin` | `deny`, default `deny`)
//! applies when no group matches. Misconfiguration fails startup rather than
//! quietly granting or denying access.

use super::jwt::Role;
use anyhow::{anyhow, bail, Result};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupMapping {
    pub group: String,
    pub role: Role,
    /// `Some` confines the user to these namespaces; `None` is unrestricted.
    pub namespaces: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefaultRole {
    Role(Role),
    Deny,
}

#[derive(Debug, Clone)]
pub struct GroupPolicy {
    pub claim: String,
    pub mappings: Vec<GroupMapping>,
    pub default: DefaultRole,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow {
        role: Role,
        namespaces: Option<Vec<String>>,
    },
    Deny,
}

#[derive(Deserialize)]
struct RawMapping {
    group: String,
    role: String,
    #[serde(default)]
    namespaces: Option<Vec<String>>,
}

fn parse_role(s: &str) -> Result<Role> {
    match s.trim().to_ascii_lowercase().as_str() {
        "admin" => Ok(Role::Admin),
        "user" => Ok(Role::User),
        "viewer" => Ok(Role::Viewer),
        other => bail!("unknown role '{other}' (use admin, user or viewer)"),
    }
}

fn rank(r: &Role) -> u8 {
    match r {
        Role::Admin => 3,
        Role::User => 2,
        Role::Viewer => 1,
    }
}

fn valid_ns(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 63
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !s.starts_with('-')
        && !s.ends_with('-')
}

impl GroupPolicy {
    /// `Ok(None)` when no mapping is configured (legacy behaviour).
    pub fn from_env() -> Result<Option<Self>> {
        let Ok(raw) = std::env::var("ZORVIA_OIDC_GROUP_ROLES") else {
            return Ok(None);
        };
        if raw.trim().is_empty() {
            return Ok(None);
        }
        let claim = std::env::var("ZORVIA_OIDC_GROUPS_CLAIM")
            .ok()
            .filter(|c| !c.trim().is_empty())
            .unwrap_or_else(|| "groups".into());
        let default = std::env::var("ZORVIA_OIDC_DEFAULT_ROLE").ok();
        Self::parse(&raw, &claim, default.as_deref()).map(Some)
    }

    pub fn parse(raw: &str, claim: &str, default: Option<&str>) -> Result<Self> {
        let rows: Vec<RawMapping> = serde_json::from_str(raw)
            .map_err(|e| anyhow!("ZORVIA_OIDC_GROUP_ROLES is not a valid JSON array: {e}"))?;
        if rows.is_empty() || rows.len() > 256 {
            bail!("ZORVIA_OIDC_GROUP_ROLES must list between 1 and 256 mappings");
        }
        let mut mappings = Vec::with_capacity(rows.len());
        for r in rows {
            if r.group.trim().is_empty() || r.group.len() > 256 {
                bail!("a group name in ZORVIA_OIDC_GROUP_ROLES is empty or too long");
            }
            let role = parse_role(&r.role)?;
            if let Some(ns) = &r.namespaces {
                if ns.len() > 64 || ns.iter().any(|n| !valid_ns(n)) {
                    bail!("group '{}' has an invalid namespace list", r.group);
                }
            }
            mappings.push(GroupMapping {
                group: r.group,
                role,
                namespaces: r.namespaces,
            });
        }
        let default = match default.map(|d| d.trim().to_ascii_lowercase()).as_deref() {
            None | Some("") | Some("deny") => DefaultRole::Deny,
            Some(r) => DefaultRole::Role(parse_role(r)?),
        };
        Ok(Self {
            claim: claim.to_string(),
            mappings,
            default,
        })
    }

    /// Role and namespaces for a user who belongs to `groups`: the highest matching
    /// role; namespaces are the union over the matching mappings *at that role*, and any
    /// of those without a list makes the user unrestricted (an Admin always is).
    pub fn decide(&self, groups: &[String]) -> Decision {
        let matched: Vec<&GroupMapping> = self
            .mappings
            .iter()
            .filter(|m| groups.contains(&m.group))
            .collect();
        if matched.is_empty() {
            return match &self.default {
                DefaultRole::Deny => Decision::Deny,
                DefaultRole::Role(r) => Decision::Allow {
                    role: r.clone(),
                    namespaces: None,
                },
            };
        }
        let role = matched
            .iter()
            .map(|m| m.role.clone())
            .max_by_key(rank)
            .expect("non-empty");
        // Only the mappings at the winning role shape the allow-list: a global read-only
        // group must not lift the restriction of a restricted user-level group.
        let at_role: Vec<&&GroupMapping> = matched.iter().filter(|m| m.role == role).collect();
        let namespaces = if role == Role::Admin || at_role.iter().any(|m| m.namespaces.is_none()) {
            None
        } else {
            let mut all: Vec<String> = at_role
                .iter()
                .flat_map(|m| m.namespaces.clone().unwrap_or_default())
                .collect();
            all.sort();
            all.dedup();
            Some(all)
        };
        Decision::Allow { role, namespaces }
    }
}

/// Group names from the ID token claim `claim`: an array of strings, or one string.
pub fn groups_from_claims(extra: &HashMap<String, Value>, claim: &str) -> Vec<String> {
    match extra.get(claim) {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        Some(Value::String(s)) if !s.is_empty() => vec![s.clone()],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn policy(default: Option<&str>) -> GroupPolicy {
        GroupPolicy::parse(
            r#"[{"group":"admins","role":"admin"},
                {"group":"devs-a","role":"user","namespaces":["team-a"]},
                {"group":"devs-b","role":"user","namespaces":["team-b"]},
                {"group":"auditors","role":"viewer"},
                {"group":"ops","role":"user"}]"#,
            "groups",
            default,
        )
        .unwrap()
    }

    fn g(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn highest_role_wins_and_admin_is_unrestricted() {
        let p = policy(None);
        assert_eq!(
            p.decide(&g(&["devs-a", "admins"])),
            Decision::Allow {
                role: Role::Admin,
                namespaces: None
            }
        );
        assert_eq!(
            p.decide(&g(&["auditors", "devs-a"])),
            Decision::Allow {
                role: Role::User,
                namespaces: Some(vec!["team-a".into()])
            }
        );
    }

    #[test]
    fn namespaces_union_and_any_unrestricted_match_lifts_the_restriction() {
        let p = policy(None);
        assert_eq!(
            p.decide(&g(&["devs-b", "devs-a"])),
            Decision::Allow {
                role: Role::User,
                namespaces: Some(vec!["team-a".into(), "team-b".into()])
            }
        );
        assert_eq!(
            p.decide(&g(&["devs-a", "ops"])),
            Decision::Allow {
                role: Role::User,
                namespaces: None
            },
            "ops has no namespace list, so the user is unrestricted"
        );
    }

    #[test]
    fn unmatched_users_follow_the_default_which_is_deny() {
        assert_eq!(policy(None).decide(&g(&["something-else"])), Decision::Deny);
        assert_eq!(policy(Some("deny")).decide(&[]), Decision::Deny);
        assert_eq!(
            policy(Some("viewer")).decide(&g(&["x"])),
            Decision::Allow {
                role: Role::Viewer,
                namespaces: None
            }
        );
    }

    #[test]
    fn group_matching_is_exact_and_case_sensitive() {
        let p = policy(None);
        assert_eq!(p.decide(&g(&["Admins"])), Decision::Deny);
        assert_eq!(p.decide(&g(&["admins-extra"])), Decision::Deny);
    }

    #[test]
    fn misconfiguration_is_rejected() {
        for bad in [
            "not json",
            "[]",
            r#"[{"group":"","role":"admin"}]"#,
            r#"[{"group":"a","role":"root"}]"#,
            r#"[{"group":"a","role":"user","namespaces":["Bad_Name"]}]"#,
            r#"{"group":"a","role":"user"}"#,
        ] {
            assert!(GroupPolicy::parse(bad, "groups", None).is_err(), "{bad}");
        }
        assert!(GroupPolicy::parse(
            r#"[{"group":"a","role":"user"}]"#,
            "groups",
            Some("superuser")
        )
        .is_err());
    }

    #[test]
    fn groups_come_from_an_array_or_a_single_string() {
        let mut m = HashMap::new();
        m.insert("groups".to_string(), json!(["a", "b", 7]));
        m.insert("roles".to_string(), json!("solo"));
        m.insert("empty".to_string(), json!(""));
        assert_eq!(groups_from_claims(&m, "groups"), vec!["a", "b"]);
        assert_eq!(groups_from_claims(&m, "roles"), vec!["solo"]);
        assert!(groups_from_claims(&m, "empty").is_empty());
        assert!(groups_from_claims(&m, "missing").is_empty());
    }
}
