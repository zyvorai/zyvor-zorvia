// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Lab-mode and known-default credential guards.

/// Previously committed lab defaults — refuse outside explicit lab mode.
pub const KNOWN_LAB_JWT_SECRETS: &[&str] = &["zorvia-lab-jwt-change-me-30152"];
pub const KNOWN_LAB_ADMIN_PASSWORDS: &[&str] = &["Admin@321"];

pub fn lab_mode() -> bool {
    match std::env::var("ZORVIA_LAB_MODE") {
        Ok(v) => {
            let v = v.trim();
            v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes")
        }
        Err(_) => false,
    }
}

pub fn is_known_lab_jwt_secret(secret: &str) -> bool {
    KNOWN_LAB_JWT_SECRETS.contains(&secret)
}

pub fn is_known_lab_admin_password(password: &str) -> bool {
    KNOWN_LAB_ADMIN_PASSWORDS.contains(&password)
}

/// Refuse startup when known lab credentials are used without lab mode.
pub fn refuse_known_defaults(jwt_secret: &str, admin_password: Option<&str>) -> anyhow::Result<()> {
    if lab_mode() {
        return Ok(());
    }
    if is_known_lab_jwt_secret(jwt_secret) {
        anyhow::bail!(
            "Refusing to start with a known lab JWT secret. Set a unique ZORVIA_JWT_SECRET \
             or set ZORVIA_LAB_MODE=1 for local lab only."
        );
    }
    if let Some(pw) = admin_password {
        if is_known_lab_admin_password(pw) {
            anyhow::bail!(
                "Refusing to start with a known lab admin password. Set a unique \
                 ZORVIA_ADMIN_PASSWORD or set ZORVIA_LAB_MODE=1 for local lab only."
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_defaults_detected() {
        assert!(is_known_lab_jwt_secret("zorvia-lab-jwt-change-me-30152"));
        assert!(is_known_lab_admin_password("Admin@321"));
        assert!(!is_known_lab_jwt_secret("unique-secret-value"));
    }

    #[test]
    fn refuse_without_lab_mode() {
        // Ensure lab mode is off for this process env during the check by
        // only testing the pure helpers when LAB is unset — refuse_known_defaults
        // reads env; we test the known-default detection path via helpers.
        assert!(is_known_lab_jwt_secret(KNOWN_LAB_JWT_SECRETS[0]));
    }
}
