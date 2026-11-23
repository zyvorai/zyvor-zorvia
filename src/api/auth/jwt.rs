// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

use anyhow::Result;
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    User,
    Viewer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub username: String,
    pub role: Role,
    pub exp: usize,
    pub jti: String,
    /// Token version — must match `users.token_version` for local accounts.
    #[serde(default)]
    pub tv: u32,
}

pub struct JwtConfig {
    secret: String,
    /// Access-token lifetime in minutes (default 60).
    expiration_minutes: i64,
}

/// Shortest accepted `ZORVIA_JWT_SECRET` outside lab mode.
pub const MIN_JWT_SECRET_LEN: usize = 32;

impl JwtConfig {
    pub fn new(secret: impl Into<String>) -> Self {
        Self {
            secret: secret.into(),
            expiration_minutes: 60,
        }
    }

    pub fn with_expiration_minutes(mut self, minutes: i64) -> Self {
        self.expiration_minutes = minutes.max(1);
        self
    }

    pub fn secret(&self) -> &str {
        &self.secret
    }

    pub fn from_env() -> Self {
        let secret = match std::env::var("ZORVIA_JWT_SECRET") {
            // An empty or short HMAC key is brute-forceable offline from any
            // issued token, so refuse it (lab mode keeps short lab secrets).
            Ok(s) if s.len() >= MIN_JWT_SECRET_LEN || super::lab_mode() => s,
            other => {
                let fallback = uuid::Uuid::new_v4().to_string();
                match other {
                    Ok(_) => log::error!(
                        "ZORVIA_JWT_SECRET is shorter than {MIN_JWT_SECRET_LEN} bytes; \
                         ignoring it and using an ephemeral secret (tokens reset on restart)"
                    ),
                    Err(_) => log::warn!(
                        "ZORVIA_JWT_SECRET unset; using ephemeral secret (tokens reset on restart)"
                    ),
                }
                fallback
            }
        };
        let minutes = std::env::var("ZORVIA_JWT_TTL_MINUTES")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(60)
            .clamp(1, 24 * 60);
        Self::new(secret).with_expiration_minutes(minutes)
    }

    pub fn generate(
        &self,
        user_id: &str,
        username: &str,
        role: Role,
        token_version: u32,
    ) -> Result<String> {
        let exp =
            (Utc::now() + Duration::minutes(self.expiration_minutes.max(1))).timestamp() as usize;
        let claims = Claims {
            sub: user_id.to_string(),
            username: username.to_string(),
            role,
            exp,
            jti: uuid::Uuid::new_v4().to_string(),
            tv: token_version,
        };
        Ok(encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(self.secret.as_bytes())?,
        )?)
    }

    pub fn validate(&self, token: &str) -> Result<Claims> {
        let data = decode::<Claims>(
            token,
            &DecodingKey::from_secret(self.secret.as_bytes())?,
            &Validation::default(),
        )?;
        Ok(data.claims)
    }
}
