// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Authentication: JWT login, bootstrap admin DB, TOTP 2FA, scoped API tokens.
//! OIDC is opt-in (`ZORVIA_OIDC_ENABLED=1`) with discovery, PKCE, token exchange, and JWKS.

mod groups;
pub mod handlers;
mod identity;
mod jwt;
mod lab_guards;
mod oidc;
pub mod permissions;
pub mod saml;
pub mod tenancy;
mod user_db;

pub use handlers::{
    change_password_handler, create_api_token_handler, delete_api_token_handler,
    list_api_tokens_handler, login_handler, logout_handler, me_handler, oidc_callback_handler,
    oidc_login_handler, providers_handler, revoke_api_token_handler, totp_disable_handler,
    totp_setup_handler, totp_verify_handler, AuthState, ChangePasswordRequest,
    CreateApiTokenRequest, LoginRequest, OidcCallbackQuery, SharedAuth, TotpDisableRequest,
    TotpVerifyRequest,
};
pub use identity::{AuthIdentity, AuthKind};
pub use jwt::{Claims, JwtConfig, Role};
pub use lab_guards::{
    lab_mode, refuse_known_defaults, KNOWN_LAB_ADMIN_PASSWORDS, KNOWN_LAB_JWT_SECRETS,
};
pub use oidc::OidcConfig;
pub use permissions::{
    permissions_for_role, required_permission, role_has_permission, ApiPermission,
};
pub use user_db::UserDb;
