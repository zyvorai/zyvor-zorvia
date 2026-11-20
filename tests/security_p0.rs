// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! P0 security regression tests (no Kubernetes required).

#[cfg(feature = "web")]
mod p0_security {
    use zorvia::api::auth::{
        permissions_for_role, refuse_known_defaults, required_permission, role_has_permission,
        ApiPermission, JwtConfig, Role, UserDb, KNOWN_LAB_ADMIN_PASSWORDS, KNOWN_LAB_JWT_SECRETS,
    };

    #[test]
    fn viewer_lacks_mutating_permissions() {
        let role = Role::Viewer;
        assert!(role_has_permission(&role, ApiPermission::VmRead));
        for p in [
            ApiPermission::VmPower,
            ApiPermission::VmCreate,
            ApiPermission::VmDelete,
            ApiPermission::StorageAdmin,
            ApiPermission::ClusterAdmin,
            ApiPermission::UsersAdmin,
        ] {
            assert!(
                !role_has_permission(&role, p),
                "viewer must not have {}",
                p.as_str()
            );
        }
    }

    #[test]
    fn user_cannot_delete_or_admin_storage() {
        let role = Role::User;
        assert!(role_has_permission(&role, ApiPermission::VmCreate));
        assert!(role_has_permission(&role, ApiPermission::VmPower));
        assert!(!role_has_permission(&role, ApiPermission::VmDelete));
        assert!(!role_has_permission(&role, ApiPermission::StorageAdmin));
        assert!(!role_has_permission(&role, ApiPermission::UsersAdmin));
        assert!(!role_has_permission(&role, ApiPermission::ClusterAdmin));
    }

    #[test]
    fn route_map_blocks_viewer_mutations() {
        let viewer = permissions_for_role(&Role::Viewer);
        let cases = [
            ("POST", "/vms"),
            ("DELETE", "/vms/demo"),
            ("POST", "/vms/demo/start"),
            ("POST", "/storage/rook/bootstrap"),
            ("POST", "/backups"),
            ("POST", "/webhooks"),
            ("POST", "/network-policies"),
            ("POST", "/vms/demo/migrate"),
        ];
        for (method, path) in cases {
            let required = required_permission(method, path).expect("mutating route");
            assert!(
                !viewer.contains(&required),
                "viewer must lack {} for {} {}",
                required.as_str(),
                method,
                path
            );
        }
    }

    #[test]
    fn oidc_from_env_off_without_enabled_flag() {
        // Without ZORVIA_OIDC_ENABLED=1, config stays None (even if other vars leak in CI).
        if std::env::var("ZORVIA_OIDC_ENABLED").ok().as_deref() == Some("1") {
            return;
        }
        assert!(zorvia::api::auth::OidcConfig::from_env().is_none());
    }

    #[test]
    fn disabled_user_and_token_version_invalidate_jwt() {
        let db = UserDb::open(":memory:").unwrap();
        let user = db
            .create_user("viewer1", "password123", Role::Viewer)
            .unwrap();
        let jwt = JwtConfig::new("test-secret-not-lab-default").with_expiration_minutes(60);
        let token = jwt
            .generate(&user.id, &user.username, Role::Viewer, user.token_version)
            .unwrap();
        let claims = jwt.validate(&token).unwrap();
        assert_eq!(claims.tv, 0);

        db.set_enabled(&user.id, false).unwrap();
        let updated = db.get_by_id(&user.id).unwrap().unwrap();
        assert!(!updated.enabled);
        assert_eq!(updated.token_version, 1);
        // Stale token version no longer matches.
        assert_ne!(claims.tv, updated.token_version);
    }

    #[test]
    fn totp_disable_bumps_token_version() {
        let db = UserDb::open(":memory:").unwrap();
        let user = db.create_user("mfa", "password123", Role::User).unwrap();
        db.set_totp(&user.id, "JBSWY3DPEHPK3PXP", true).unwrap();
        assert_eq!(db.get_by_id(&user.id).unwrap().unwrap().token_version, 0);
        db.disable_totp(&user.id).unwrap();
        let u = db.get_by_id(&user.id).unwrap().unwrap();
        assert!(!u.totp_enabled);
        assert_eq!(u.token_version, 1);
    }

    #[test]
    fn known_lab_defaults_refused_outside_lab_mode() {
        // Only valid when ZORVIA_LAB_MODE is unset/false in the test process.
        if std::env::var("ZORVIA_LAB_MODE").ok().as_deref() == Some("1") {
            return;
        }
        let err =
            refuse_known_defaults(KNOWN_LAB_JWT_SECRETS[0], Some(KNOWN_LAB_ADMIN_PASSWORDS[0]));
        assert!(err.is_err());
        let ok = refuse_known_defaults("unique-production-jwt-secret-value", Some("UniquePass!99"));
        assert!(ok.is_ok());
    }

    #[test]
    fn api_token_hashed_lookup_and_role_scopes() {
        let db = UserDb::open(":memory:").unwrap();
        let (rec, plain) = db
            .create_api_token(
                "ci-read",
                Role::User,
                vec!["vm.read".into()],
                None,
                Some("admin"),
            )
            .unwrap();
        assert!(plain.starts_with("zrv_"));
        let found = db.lookup_api_token(&plain).unwrap().unwrap();
        assert_eq!(found.id, rec.id);
        assert_eq!(found.scopes, vec!["vm.read".to_string()]);
        // Wrong plaintext must not match.
        assert!(db.lookup_api_token("zrv_wrong").unwrap().is_none());
    }

    #[test]
    fn webhook_rejects_private_literals() {
        use zorvia::api::webhooks::WebhookConfig;
        assert!(WebhookConfig::new("x", "https://127.0.0.1/h").is_err());
        assert!(WebhookConfig::new("x", "https://10.0.0.5/h").is_err());
        assert!(WebhookConfig::new("x", "http://example.com/h").is_err());
    }
}
