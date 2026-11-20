// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Authenticated request identity (JWT user or scoped API token).

use super::jwt::Role;
use super::permissions::{permissions_for_role, permissions_for_token, ApiPermission};

#[derive(Debug, Clone)]
pub enum AuthKind {
    Jwt,
    ApiToken,
    /// Legacy shared env key — only when ZORVIA_LAB_MODE=1.
    LabApiKey,
}

#[derive(Debug, Clone)]
pub struct AuthIdentity {
    pub kind: AuthKind,
    pub role: Role,
    pub permissions: Vec<ApiPermission>,
    pub user_id: Option<String>,
    pub username: Option<String>,
    pub token_id: Option<String>,
    /// `Some` confines the identity to these namespaces (see `tenancy`).
    pub namespaces: Option<Vec<String>>,
}

impl AuthIdentity {
    pub fn from_jwt(user_id: String, username: String, role: Role) -> Self {
        let permissions = permissions_for_role(&role);
        Self {
            kind: AuthKind::Jwt,
            role,
            permissions,
            user_id: Some(user_id),
            username: Some(username),
            token_id: None,
            namespaces: None,
        }
    }

    pub fn from_api_token(token_id: String, name: String, role: Role, scopes: &[String]) -> Self {
        let permissions = permissions_for_token(&role, scopes);
        Self {
            kind: AuthKind::ApiToken,
            role,
            permissions,
            user_id: None,
            username: Some(name),
            token_id: Some(token_id),
            namespaces: None,
        }
    }

    /// Lab shared API key: treated as Admin for local labs only.
    pub fn lab_api_key() -> Self {
        let role = Role::Admin;
        let permissions = permissions_for_role(&role);
        Self {
            kind: AuthKind::LabApiKey,
            role,
            permissions,
            user_id: None,
            username: Some("lab-api-key".into()),
            token_id: None,
            namespaces: None,
        }
    }

    pub fn has_permission(&self, required: ApiPermission) -> bool {
        self.permissions.contains(&required)
    }
}
