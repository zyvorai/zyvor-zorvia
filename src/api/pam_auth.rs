// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

use serde::{Deserialize, Serialize};

/// PAM authentication configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PamConfig {
    /// PAM service name (e.g., "zorvia", "login")
    pub service_name: String,
    /// Whether PAM auth is enabled
    pub enabled: bool,
    /// Allowed PAM groups (empty = all groups allowed)
    pub allowed_groups: Vec<String>,
    /// Session timeout in seconds
    pub session_timeout_secs: u64,
}

impl PamConfig {
    pub fn new(service_name: impl Into<String>) -> Self {
        Self {
            service_name: service_name.into(),
            enabled: false,
            allowed_groups: Vec::new(),
            session_timeout_secs: 3600,
        }
    }

    pub fn with_groups(mut self, groups: Vec<String>) -> Self {
        self.allowed_groups = groups;
        self
    }

    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.session_timeout_secs = secs;
        self
    }

    pub fn enable(mut self) -> Self {
        self.enabled = true;
        self
    }
}

impl Default for PamConfig {
    fn default() -> Self {
        Self::new("zorvia")
    }
}

/// PAM authentication result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PamAuthResult {
    pub authenticated: bool,
    pub username: String,
    pub groups: Vec<String>,
    pub error: Option<String>,
}

impl PamAuthResult {
    pub fn success(username: impl Into<String>, groups: Vec<String>) -> Self {
        Self {
            authenticated: true,
            username: username.into(),
            groups,
            error: None,
        }
    }

    pub fn failure(username: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            authenticated: false,
            username: username.into(),
            groups: Vec::new(),
            error: Some(error.into()),
        }
    }
}

/// Validate username format for PAM authentication.
pub fn validate_username(username: &str) -> bool {
    if username.is_empty() || username.len() > 32 {
        return false;
    }
    username
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
}

/// Check whether a user belongs to any of the allowed groups.
pub fn check_group_membership(user_groups: &[String], allowed_groups: &[String]) -> bool {
    if allowed_groups.is_empty() {
        return true;
    }
    user_groups.iter().any(|g| allowed_groups.contains(g))
}

/// Authenticate against host PAM when `ZORVIA_PAM=1`.
///
/// Without the `pam` feature / on non-Linux, returns `Ok(false)` so login falls
/// through to DB-only credentials.
pub fn authenticate_pam(username: &str, password: &str) -> anyhow::Result<bool> {
    if std::env::var("ZORVIA_PAM").ok().as_deref() != Some("1") {
        return Ok(false);
    }
    if !validate_username(username) || password.is_empty() {
        return Ok(false);
    }

    #[cfg(all(feature = "pam", target_os = "linux"))]
    {
        // Real PAM requires linking libpam; enabled via --features pam on Linux builds.
        // Placeholder until pam crate is added to Cargo features with system deps.
        log::warn!("PAM feature requested but not fully linked; rejecting PAM login");
        return Ok(false);
    }

    #[cfg(not(all(feature = "pam", target_os = "linux")))]
    {
        log::debug!("PAM not available on this build; skipping");
        let _ = (username, password);
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pam_config_defaults() {
        let config = PamConfig::default();
        assert_eq!(config.service_name, "zorvia");
        assert!(!config.enabled);
    }

    #[test]
    fn test_validate_username() {
        assert!(validate_username("admin"));
        assert!(!validate_username(""));
        assert!(!validate_username(&"a".repeat(40)));
    }

    #[test]
    fn test_pam_auth_result() {
        let failure = PamAuthResult::failure("admin", "bad password");
        assert!(!failure.authenticated);
    }
}
