use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Redirect},
    Json,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use totp_rs::{Algorithm, Builder, Secret};

use super::identity::AuthIdentity;
use super::jwt::{Claims, JwtConfig, Role};
use super::lab_guards::{lab_mode, refuse_known_defaults};
use super::oidc::{
    build_authorize_url, discover, display_username, exchange_code, pkce_challenge_s256,
    random_urlsafe, verify_id_token, OidcConfig, PendingOidc, ProviderInfo,
};
use super::user_db::{ApiTokenRecord, UserDb};
use crate::api::pam_auth::{authenticate_pam, validate_username};

const OIDC_PENDING_TTL_SECS: u64 = 600;

/// Bring an existing OIDC user in line with what the IdP groups now say. A change of role or
/// namespaces bumps `token_version`, which revokes the user's other sessions (so a demotion
/// or a narrowed allow-list cannot be outlived by an old token). Returns the refreshed user.
fn sync_oidc_user(
    db: &UserDb,
    user: super::user_db::User,
    role: Role,
    namespaces: Option<Vec<String>>,
) -> anyhow::Result<super::user_db::User> {
    // Admins are never namespace-restricted.
    let namespaces = if role == Role::Admin {
        None
    } else {
        namespaces
    };
    let current = db.get_namespaces(&user.id)?;
    let mut changed = false;
    if user.role != role {
        db.update_role(&user.id, role)?; // bumps token_version
        changed = true;
    }
    if current != namespaces {
        db.set_namespaces(&user.id, namespaces.as_deref())?;
        if !changed {
            db.bump_token_version(&user.id)?;
        }
        changed = true;
    }
    if changed {
        return db
            .get_by_id(&user.id)?
            .ok_or_else(|| anyhow::anyhow!("user vanished during group sync"));
    }
    Ok(user)
}

/// Write the generated bootstrap password to `<dir>/bootstrap-admin-password`
/// with owner-only permissions (never overwriting an existing file's mode).
fn write_bootstrap_password(
    dir: &std::path::Path,
    user: &str,
    password: &str,
) -> std::io::Result<std::path::PathBuf> {
    use std::io::Write;
    std::fs::create_dir_all(dir)?;
    let path = dir.join("bootstrap-admin-password");
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&path)?;
    writeln!(f, "user: {user}\npassword: {password}")?;
    Ok(path)
}

pub struct AuthState {
    pub jwt: JwtConfig,
    pub db: UserDb,
    /// Lab-only shared API key (honored only when ZORVIA_LAB_MODE=1).
    pub lab_api_key: Option<String>,
    pub oidc: Option<OidcConfig>,
    /// OIDC group -> role mapping; `None` keeps the legacy behaviour (every OIDC user is a User).
    pub oidc_groups: Option<super::groups::GroupPolicy>,
    pub oidc_pending: Mutex<HashMap<String, PendingOidc>>,
}

impl AuthState {
    pub fn from_env() -> anyhow::Result<Self> {
        // Key provider first: it protects the TOTP secrets in the database opened next.
        crate::keys::init_from_env()?;
        let db = UserDb::from_env()?;
        // Moving from SQLite to PostgreSQL: with ZORVIA_DATABASE_IMPORT=1 an empty PostgreSQL
        // store is filled once from the existing auth.db (the file is left untouched).
        if db.is_postgres()
            && std::env::var("ZORVIA_DATABASE_IMPORT").is_ok_and(|v| v == "1")
            && db.count_users()? == 0
        {
            let src = UserDb::env_path();
            if std::path::Path::new(&src).exists() {
                let n = db.import_from_sqlite(&src)?;
                log::info!("imported {n} user(s) from {src} into PostgreSQL");
            }
        }
        match db.seal_existing_totp() {
            Ok(n) if n > 0 => log::info!("sealed {n} TOTP secret(s) with the key provider"),
            Ok(_) => {}
            Err(e) => anyhow::bail!("cannot seal existing TOTP secrets: {e:#}"),
        }
        let admin_user = std::env::var("ZORVIA_ADMIN_USER").unwrap_or_else(|_| "admin".to_string());

        let admin_password = if lab_mode() {
            std::env::var("ZORVIA_ADMIN_PASSWORD").unwrap_or_else(|_| "Admin@321".to_string())
        } else if let Ok(pw) = std::env::var("ZORVIA_ADMIN_PASSWORD") {
            pw
        } else if db.count_users()? == 0 {
            // First boot outside lab: generate a one-time password.
            let generated = format!("Zv-{}", uuid::Uuid::new_v4());
            // Not into the log (it is shipped and retained): a 0600 file next to the
            // auth database, to be read once and deleted.
            let dir = std::path::Path::new(&UserDb::env_path())
                .parent()
                .map(std::path::Path::to_path_buf)
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            match write_bootstrap_password(&dir, &admin_user, &generated) {
                Ok(path) => log::warn!(
                    "ZORVIA_ADMIN_PASSWORD unset; generated a bootstrap password for '{}' and \
                     wrote it to {} (mode 0600). Read it once, delete the file, then rotate it.",
                    admin_user,
                    path.display()
                ),
                Err(e) => log::warn!(
                    "ZORVIA_ADMIN_PASSWORD unset; generated bootstrap password for '{}': {} \
                     (could not write it to a file: {e}; shown once, store it and rotate)",
                    admin_user,
                    generated
                ),
            }
            generated
        } else {
            // DB already seeded; password only needed for seed.
            String::new()
        };

        let jwt = JwtConfig::from_env();
        refuse_known_defaults(
            jwt.secret(),
            if admin_password.is_empty() {
                None
            } else {
                Some(admin_password.as_str())
            },
        )?;

        if !admin_password.is_empty() {
            db.seed_admin(&admin_user, &admin_password)?;
        }

        let lab_api_key = match std::env::var("ZORVIA_API_KEY") {
            Ok(k) if !k.is_empty() => {
                if lab_mode() {
                    log::warn!(
                        "ZORVIA_API_KEY is active in lab mode only; prefer scoped /v1/api-tokens"
                    );
                    Some(k)
                } else {
                    log::warn!(
                        "ZORVIA_API_KEY is set but ignored outside ZORVIA_LAB_MODE=1; \
                         create scoped tokens via POST /api/v1/api-tokens"
                    );
                    None
                }
            }
            _ => None,
        };

        Ok(Self {
            jwt,
            db,
            lab_api_key,
            oidc: OidcConfig::from_env(),
            // A broken mapping fails startup: guessing would grant or deny access wrongly.
            oidc_groups: super::groups::GroupPolicy::from_env()?,
            oidc_pending: Mutex::new(HashMap::new()),
        })
    }

    /// Validate JWT and ensure local users are still enabled with matching token_version.
    pub fn validate_bearer(&self, token: &str) -> Option<Claims> {
        self.validate_bearer_checked(token).ok().flatten()
    }

    /// Like `validate_bearer`, but tells "this credential is not valid" (`Ok(None)`) apart
    /// from "the user store could not be read" (`Err`), so an outage of the database is not
    /// reported to clients as a revoked session.
    pub fn validate_bearer_checked(&self, token: &str) -> anyhow::Result<Option<Claims>> {
        let Ok(claims) = self.jwt.validate(token) else {
            return Ok(None);
        };
        // Non-local identities (PAM) have no DB row / token_version.
        // Their `tv` is the issue time, so logout can revoke by timestamp.
        if let Some(name) = claims.sub.strip_prefix("pam:") {
            if claims.tv < self.db.pam_not_before(name)? {
                return Ok(None);
            }
            return Ok(Some(claims));
        }
        let Some(user) = self.db.get_by_id(&claims.sub)? else {
            return Ok(None);
        };
        if !user.enabled || user.token_version != claims.tv {
            return Ok(None);
        }
        Ok(Some(claims))
    }

    pub fn lab_api_key_ok(&self, provided: &str) -> bool {
        match &self.lab_api_key {
            Some(expected) => {
                use subtle::ConstantTimeEq;
                bool::from(provided.as_bytes().ct_eq(expected.as_bytes()))
            }
            None => false,
        }
    }

    /// Resolve a credential string (Bearer or x-api-key) into an identity.
    pub fn resolve_credential(&self, credential: &str) -> Option<AuthIdentity> {
        match self.check_credential(credential) {
            CredentialCheck::Valid(id) => Some(id),
            _ => None,
        }
    }

    /// Resolve a credential, distinguishing a store outage from an invalid credential.
    /// Still fails closed: nothing is authenticated while the store cannot be read.
    pub fn check_credential(&self, credential: &str) -> CredentialCheck {
        if self.lab_api_key_ok(credential) {
            return CredentialCheck::Valid(AuthIdentity::lab_api_key());
        }
        match self.db.lookup_api_token(credential) {
            Ok(Some(tok)) => {
                let _ = self.db.touch_api_token(&tok.id);
                let mut id = AuthIdentity::from_api_token(tok.id, tok.name, tok.role, &tok.scopes);
                id.namespaces = tok.namespaces;
                return CredentialCheck::Valid(id);
            }
            Ok(None) => {}
            Err(e) => {
                log::warn!("auth store unavailable: {e}");
                return CredentialCheck::StoreUnavailable;
            }
        }
        let claims = match self.validate_bearer_checked(credential) {
            Ok(Some(c)) => c,
            Ok(None) => return CredentialCheck::Invalid,
            Err(e) => {
                log::warn!("auth store unavailable: {e}");
                return CredentialCheck::StoreUnavailable;
            }
        };
        let mut id = AuthIdentity::from_jwt(claims.sub.clone(), claims.username, claims.role);
        if !claims.sub.starts_with("pam:") && !matches!(id.role, Role::Admin) {
            // Fail closed: if the allow-list cannot be read, confine to nothing.
            id.namespaces = match self.db.get_namespaces(&claims.sub) {
                Ok(n) => n,
                Err(_) => Some(Vec::new()),
            };
        }
        CredentialCheck::Valid(id)
    }
}

/// Outcome of resolving a credential.
#[derive(Debug)]
pub enum CredentialCheck {
    Valid(AuthIdentity),
    /// Unknown, expired, revoked or malformed.
    Invalid,
    /// The user store could not be read (for example PostgreSQL is down).
    StoreUnavailable,
}

pub type SharedAuth = Arc<AuthState>;

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
    pub totp_code: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub user_id: String,
    pub role: String,
    pub username: String,
}

fn role_str(r: &Role) -> &'static str {
    match r {
        Role::Admin => "admin",
        Role::User => "user",
        Role::Viewer => "viewer",
    }
}

fn err(status: StatusCode, msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    (
        status,
        Json(serde_json::json!({
            "success": false,
            "status": status.as_u16(),
            "error": { "code": status.as_str(), "message": msg },
            "data": null
        })),
    )
}

/// Failed-login throttle: after `MAX_FAILURES` 401s for one username inside
/// `LOCKOUT`, further attempts get 429 until the window ends. This also caps
/// TOTP guessing, because a wrong code is a 401. In memory per process, plus a
/// shared counter in the user database on PostgreSQL (see `shared_throttle`),
/// keyed by lower-cased username (unknown names are tracked too, so the
/// response does not reveal whether an account exists).
mod login_throttle {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    pub const MAX_FAILURES: u32 = 8;
    pub const LOCKOUT: Duration = Duration::from_secs(15 * 60);
    const MAX_TRACKED: usize = 10_000;

    static FAILS: Mutex<Option<HashMap<String, (u32, Instant)>>> = Mutex::new(None);

    pub fn blocked(user: &str) -> bool {
        let mut g = FAILS.lock().unwrap_or_else(|e| e.into_inner());
        let m = g.get_or_insert_with(HashMap::new);
        match m.get(&user.to_lowercase()) {
            Some((n, at)) if at.elapsed() < LOCKOUT => *n >= MAX_FAILURES,
            Some(_) => {
                m.remove(&user.to_lowercase());
                false
            }
            None => false,
        }
    }

    pub fn record_failure(user: &str) {
        let mut g = FAILS.lock().unwrap_or_else(|e| e.into_inner());
        let m = g.get_or_insert_with(HashMap::new);
        if m.len() >= MAX_TRACKED {
            m.retain(|_, (_, at)| at.elapsed() < LOCKOUT);
        }
        let e = m.entry(user.to_lowercase()).or_insert((0, Instant::now()));
        if e.1.elapsed() >= LOCKOUT {
            *e = (0, Instant::now());
        }
        e.0 += 1;
        // Every further failure extends the lockout.
        e.1 = Instant::now();
    }

    pub fn clear(user: &str) {
        let mut g = FAILS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(m) = g.as_mut() {
            m.remove(&user.to_lowercase());
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn locks_after_repeated_failures_and_clears() {
            let u = "throttle-test-user";
            assert!(!blocked(u));
            for _ in 0..MAX_FAILURES {
                record_failure(u);
            }
            assert!(blocked(u));
            assert!(blocked("THROTTLE-TEST-USER"));
            clear(u);
            assert!(!blocked(u));
        }
    }
}

/// The in-memory counter always applies; on PostgreSQL the shared counter applies as well, so
/// attempts spread over several replicas still add up to one lockout.
async fn shared_throttle<T: Send + 'static>(
    auth: &SharedAuth,
    f: impl FnOnce(&super::user_db::UserDb, i64) -> anyhow::Result<T> + Send + 'static,
) -> Option<T> {
    if !auth.db.is_postgres() {
        return None;
    }
    let auth = auth.clone();
    let now = chrono::Utc::now().timestamp();
    match tokio::task::spawn_blocking(move || f(&auth.db, now)).await {
        Ok(Ok(v)) => Some(v),
        Ok(Err(e)) => {
            log::warn!("shared login throttle unavailable: {e}");
            None
        }
        Err(_) => None,
    }
}

pub async fn login_handler(
    State(auth): State<SharedAuth>,
    Json(req): Json<LoginRequest>,
) -> axum::response::Response {
    let username = req.username.clone();
    let key = username.to_lowercase();
    let lockout = login_throttle::LOCKOUT.as_secs() as i64;
    let max = login_throttle::MAX_FAILURES as i64;
    let shared_blocked = {
        let k = key.clone();
        shared_throttle(&auth, move |db, now| {
            db.login_blocked(&k, now, max, lockout)
        })
        .await
        .unwrap_or(false)
    };
    if shared_blocked || login_throttle::blocked(&username) {
        return err(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many failed sign-in attempts. Try again later.",
        )
        .into_response();
    }
    let resp = login_inner(auth.clone(), req).await;
    match resp.status() {
        StatusCode::UNAUTHORIZED => {
            login_throttle::record_failure(&username);
            shared_throttle(&auth, move |db, now| {
                db.login_record_failure(&key, now, lockout)
            })
            .await;
        }
        s if s.is_success() => {
            login_throttle::clear(&username);
            shared_throttle(&auth, move |db, _| db.login_clear(&key)).await;
        }
        _ => {}
    }
    resp
}

async fn login_inner(auth: SharedAuth, req: LoginRequest) -> axum::response::Response {
    if !validate_username(&req.username) {
        return err(StatusCode::BAD_REQUEST, "Invalid username format").into_response();
    }

    if let Ok(Some(user)) = auth.db.get_by_username(&req.username) {
        match verify_password_off_runtime(user.password_hash.clone(), req.password.clone()).await {
            Ok(true) => {
                // Only someone who knows the password learns the account is
                // disabled; a wrong password is a plain 401 either way.
                if !user.enabled {
                    return err(StatusCode::FORBIDDEN, "This account has been disabled")
                        .into_response();
                }
                if user.totp_enabled {
                    let Some(code) = req.totp_code.as_deref() else {
                        return (
                            StatusCode::FORBIDDEN,
                            Json(serde_json::json!({
                                "success": false,
                                "status": 403,
                                "error": {
                                    "code": "requires_2fa",
                                    "message": "2FA code required",
                                    "requires_2fa": true
                                },
                                "data": null
                            })),
                        )
                            .into_response();
                    };
                    if !check_and_consume_totp(&auth, &user, code) {
                        return err(StatusCode::UNAUTHORIZED, "Invalid 2FA code").into_response();
                    }
                }
                return match auth.jwt.generate(
                    &user.id,
                    &user.username,
                    user.role.clone(),
                    user.token_version,
                ) {
                    Ok(token) => {
                        let _ = auth.db.update_last_login(&user.id);
                        Json(LoginResponse {
                            token,
                            user_id: user.id,
                            role: role_str(&user.role).into(),
                            username: user.username,
                        })
                        .into_response()
                    }
                    Err(_) => err(StatusCode::INTERNAL_SERVER_ERROR, "Token error").into_response(),
                };
            }
            Ok(false) => {
                return err(StatusCode::UNAUTHORIZED, "Invalid credentials").into_response();
            }
            Err(_) => {
                return err(StatusCode::INTERNAL_SERVER_ERROR, "Auth error").into_response();
            }
        }
    }

    // Unknown local user: spend the same bcrypt time a real check would, so
    // response timing does not reveal which usernames exist.
    let burn_password = req.password.clone();
    let _ = tokio::task::spawn_blocking(move || burn_password_check(&burn_password)).await;

    let username = req.username.clone();
    let password = req.password.clone();
    let pam_ok = tokio::task::spawn_blocking(move || authenticate_pam(&username, &password))
        .await
        .unwrap_or(Ok(false))
        .unwrap_or(false);

    if pam_ok {
        let uid = format!("pam:{}", req.username);
        return match auth
            .jwt
            .generate(&uid, &req.username, Role::User, pam_now())
        {
            Ok(token) => Json(LoginResponse {
                token,
                user_id: uid,
                role: "user".into(),
                username: req.username,
            })
            .into_response(),
            Err(_) => err(StatusCode::INTERNAL_SERVER_ERROR, "Token error").into_response(),
        };
    }

    err(StatusCode::UNAUTHORIZED, "Invalid credentials").into_response()
}

/// Verify a password against its bcrypt hash on the blocking pool.
///
/// bcrypt is deliberately slow (about a second of CPU at the default cost on a small
/// pod). Running it inline in an async handler pins a tokio worker for that long, so a
/// burst of logins starved every other request (a plain list took ~11 s behind ten
/// concurrent logins).
async fn verify_password_off_runtime(hash: String, password: String) -> anyhow::Result<bool> {
    tokio::task::spawn_blocking(move || {
        bcrypt::verify(password, &hash).map_err(anyhow::Error::from)
    })
    .await
    .map_err(|e| anyhow::anyhow!("password check task failed: {e}"))?
}

/// bcrypt of a throwaway value, computed once, to equalise timing for unknown users.
fn burn_password_check(password: &str) {
    static DUMMY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let hash = DUMMY.get_or_init(|| {
        bcrypt::hash("zorvia-timing-equaliser", bcrypt::DEFAULT_COST).unwrap_or_default()
    });
    let _ = bcrypt::verify(password, hash);
}

fn verify_totp(secret: Option<&str>, code: &str) -> bool {
    verify_totp_step(secret, code).is_some()
}

/// A valid code for `user` that has not been accepted before (RFC 6238 §5.2).
/// The accepted time step is stored, so replaying the same code, even inside its
/// ~90 s window, fails.
fn check_and_consume_totp(auth: &AuthState, user: &super::user_db::User, code: &str) -> bool {
    match verify_totp_step(user.totp_secret.as_deref(), code) {
        Some(step) => auth.db.consume_totp_step(&user.id, step).unwrap_or(false),
        None => false,
    }
}

fn verify_totp_step(secret: Option<&str>, code: &str) -> Option<u64> {
    let secret = secret?;
    let Ok(secret) = Secret::try_from_base32(secret) else {
        return None;
    };
    let Ok(totp) = Builder::new()
        .with_algorithm(Algorithm::SHA1)
        .with_digits(6)
        .with_skew(1)
        .with_step_duration(30)
        .with_secret(secret)
        .with_account_name("")
        .build()
    else {
        return None;
    };
    totp.check_current(code)
}

pub async fn me_handler(State(auth): State<SharedAuth>, headers: HeaderMap) -> impl IntoResponse {
    let Some(token) = bearer_token(&headers) else {
        return err(StatusCode::UNAUTHORIZED, "Missing bearer token").into_response();
    };
    match auth.validate_bearer(&token) {
        Some(c) => Json(serde_json::json!({
            "id": c.sub,
            "username": c.username,
            "role": role_str(&c.role),
        }))
        .into_response(),
        None => err(StatusCode::UNAUTHORIZED, "Invalid token").into_response(),
    }
}

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer ").map(|s| s.to_string()))
}

fn parse_role(s: &str) -> Option<Role> {
    match s.to_ascii_lowercase().as_str() {
        "admin" => Some(Role::Admin),
        "user" => Some(Role::User),
        "viewer" => Some(Role::Viewer),
        _ => None,
    }
}

#[derive(Debug, Serialize)]
pub struct UserSummary {
    pub id: String,
    pub username: String,
    pub role: String,
    pub enabled: bool,
    pub created: String,
    pub last_login: Option<String>,
}

impl From<&crate::api::auth::user_db::User> for UserSummary {
    fn from(u: &crate::api::auth::user_db::User) -> Self {
        Self {
            id: u.id.clone(),
            username: u.username.clone(),
            role: role_str(&u.role).to_string(),
            enabled: u.enabled,
            created: u.created.clone(),
            last_login: u.last_login.clone(),
        }
    }
}

pub async fn list_users_handler(
    State(auth): State<SharedAuth>,
    _headers: HeaderMap,
) -> impl IntoResponse {
    match auth.db.list_users() {
        Ok(users) => Json(users.iter().map(UserSummary::from).collect::<Vec<_>>()).into_response(),
        Err(_) => err(StatusCode::INTERNAL_SERVER_ERROR, "Failed to list users").into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    pub role: String,
}

pub async fn create_user_handler(
    State(auth): State<SharedAuth>,
    _headers: HeaderMap,
    Json(body): Json<CreateUserRequest>,
) -> impl IntoResponse {
    if !validate_username(&body.username) {
        return err(StatusCode::BAD_REQUEST, "Invalid username format").into_response();
    }
    if body.password.len() < 8 {
        return err(
            StatusCode::BAD_REQUEST,
            "Password must be at least 8 characters",
        )
        .into_response();
    }
    let Some(role) = parse_role(&body.role) else {
        return err(
            StatusCode::BAD_REQUEST,
            "Invalid role (expected admin, user, or viewer)",
        )
        .into_response();
    };
    if auth
        .db
        .get_by_username(&body.username)
        .ok()
        .flatten()
        .is_some()
    {
        return err(StatusCode::CONFLICT, "Username already exists").into_response();
    }
    match auth.db.create_user(&body.username, &body.password, role) {
        Ok(user) => (StatusCode::CREATED, Json(UserSummary::from(&user))).into_response(),
        Err(_) => err(StatusCode::INTERNAL_SERVER_ERROR, "Failed to create user").into_response(),
    }
}

fn reject_if_last_admin(
    auth: &SharedAuth,
    target: &crate::api::auth::user_db::User,
    target_will_remain_enabled_admin: bool,
) -> Option<axum::response::Response> {
    if target.role != Role::Admin || !target.enabled || target_will_remain_enabled_admin {
        return None;
    }
    match auth.db.enabled_admin_count() {
        Ok(n) if n <= 1 => Some(
            err(
                StatusCode::CONFLICT,
                "This is the last enabled admin account",
            )
            .into_response(),
        ),
        Ok(_) => None,
        Err(_) => Some(
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to check admin count",
            )
            .into_response(),
        ),
    }
}

pub async fn delete_user_handler(
    State(auth): State<SharedAuth>,
    _headers: HeaderMap,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let Ok(Some(target)) = auth.db.get_by_id(&id) else {
        return err(StatusCode::NOT_FOUND, "User not found").into_response();
    };
    if let Some(resp) = reject_if_last_admin(&auth, &target, false) {
        return resp;
    }
    match auth.db.delete_user(&id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => err(StatusCode::INTERNAL_SERVER_ERROR, "Failed to delete user").into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct UpdateRoleRequest {
    pub role: String,
}

pub async fn update_role_handler(
    State(auth): State<SharedAuth>,
    _headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<UpdateRoleRequest>,
) -> impl IntoResponse {
    let Some(new_role) = parse_role(&body.role) else {
        return err(StatusCode::BAD_REQUEST, "Invalid role").into_response();
    };
    let Ok(Some(target)) = auth.db.get_by_id(&id) else {
        return err(StatusCode::NOT_FOUND, "User not found").into_response();
    };
    if let Some(resp) = reject_if_last_admin(&auth, &target, new_role == Role::Admin) {
        return resp;
    }
    match auth.db.update_role(&id, new_role) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => err(StatusCode::INTERNAL_SERVER_ERROR, "Failed to update role").into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct SetEnabledRequest {
    pub enabled: bool,
}

pub async fn set_enabled_handler(
    State(auth): State<SharedAuth>,
    _headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<SetEnabledRequest>,
) -> impl IntoResponse {
    let Ok(Some(target)) = auth.db.get_by_id(&id) else {
        return err(StatusCode::NOT_FOUND, "User not found").into_response();
    };
    if let Some(resp) = reject_if_last_admin(&auth, &target, body.enabled) {
        return resp;
    }
    match auth.db.set_enabled(&id, body.enabled) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => err(StatusCode::INTERNAL_SERVER_ERROR, "Failed to update user").into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct TotpVerifyRequest {
    pub code: String,
}

#[derive(Debug, Deserialize)]
pub struct TotpDisableRequest {
    pub password: String,
    pub totp_code: String,
}

/// Body for `POST /v1/auth/totp/setup`. Only needed when 2FA is already on.
#[derive(Debug, Default, Deserialize)]
pub struct TotpSetupRequest {
    pub password: Option<String>,
    pub totp_code: Option<String>,
}

pub async fn totp_setup_handler(
    State(auth): State<SharedAuth>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let Some(claims) = bearer_token(&headers).and_then(|t| auth.validate_bearer(&t)) else {
        return err(StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    };
    // Re-enrolling replaces the secret. While 2FA is on, a bearer token alone
    // must not be able to do that, or a stolen session could swap in its own
    // authenticator and then drop the second factor.
    let mut was_enabled = false;
    if let Ok(Some(user)) = auth.db.get_by_id(&claims.sub) {
        if user.totp_enabled {
            was_enabled = true;
            let req: TotpSetupRequest = serde_json::from_slice(&body).unwrap_or_default();
            let password_ok = req
                .password
                .as_deref()
                .is_some_and(|p| user.verify_password(p).unwrap_or(false));
            let code_ok = req
                .totp_code
                .as_deref()
                .is_some_and(|c| check_and_consume_totp(&auth, &user, c));
            if !(password_ok && code_ok) {
                return err(
                    StatusCode::FORBIDDEN,
                    "2FA is already enabled: supply password and a current totp_code to re-enrol",
                )
                .into_response();
            }
        }
    }
    let secret = Secret::generate();
    let encoded = secret.to_base32();
    let Ok(totp) = Builder::new()
        .with_algorithm(Algorithm::SHA1)
        .with_digits(6)
        .with_skew(1)
        .with_step_duration(30)
        .with_secret(secret)
        .with_issuer(Some("Zorvia"))
        .with_account_name(claims.username.clone())
        .build()
    else {
        return err(StatusCode::INTERNAL_SERVER_ERROR, "TOTP error").into_response();
    };
    // While 2FA is on, the new secret waits as "pending" and the current one
    // keeps protecting the account until the new one produces a valid code.
    // Otherwise (2FA not yet on) enrolment just stores the secret as before.
    if was_enabled {
        let _ = auth.db.set_totp_pending(&claims.sub, Some(&encoded));
    } else {
        let _ = auth.db.set_totp(&claims.sub, &encoded, false);
    }
    let otpauth_url = match totp.to_url() {
        Ok(u) => u,
        Err(_) => {
            return err(StatusCode::INTERNAL_SERVER_ERROR, "TOTP error").into_response();
        }
    };
    Json(serde_json::json!({
        "secret": encoded,
        "otpauth_url": otpauth_url,
    }))
    .into_response()
}

pub async fn totp_verify_handler(
    State(auth): State<SharedAuth>,
    headers: HeaderMap,
    Json(body): Json<TotpVerifyRequest>,
) -> impl IntoResponse {
    let Some(claims) = bearer_token(&headers).and_then(|t| auth.validate_bearer(&t)) else {
        return err(StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    };
    let Ok(Some(user)) = auth.db.get_by_id(&claims.sub) else {
        return err(StatusCode::NOT_FOUND, "User not found").into_response();
    };
    if let Ok(Some(pending)) = auth.db.get_totp_pending(&user.id) {
        if !verify_totp(Some(&pending), &body.code) {
            return err(StatusCode::UNAUTHORIZED, "Invalid 2FA code").into_response();
        }
        let _ = auth.db.commit_totp_pending(&user.id, &pending);
        return Json(serde_json::json!({
            "success": true, "totp_enabled": true, "sessions_revoked": true
        }))
        .into_response();
    }
    let secret = user.totp_secret.as_deref().unwrap_or("");
    if !verify_totp(Some(secret), &body.code) {
        return err(StatusCode::UNAUTHORIZED, "Invalid 2FA code").into_response();
    }
    let _ = auth.db.set_totp(&user.id, secret, true);
    Json(serde_json::json!({ "success": true, "totp_enabled": true })).into_response()
}

pub async fn totp_disable_handler(
    State(auth): State<SharedAuth>,
    headers: HeaderMap,
    Json(body): Json<TotpDisableRequest>,
) -> impl IntoResponse {
    let Some(claims) = bearer_token(&headers).and_then(|t| auth.validate_bearer(&t)) else {
        return err(StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    };
    let Ok(Some(user)) = auth.db.get_by_id(&claims.sub) else {
        return err(StatusCode::NOT_FOUND, "User not found").into_response();
    };
    match verify_password_off_runtime(user.password_hash.clone(), body.password.clone()).await {
        Ok(true) => {}
        Ok(false) => {
            return err(StatusCode::UNAUTHORIZED, "Invalid password").into_response();
        }
        Err(_) => {
            return err(StatusCode::INTERNAL_SERVER_ERROR, "Auth error").into_response();
        }
    }
    if !user.totp_enabled {
        return err(StatusCode::BAD_REQUEST, "TOTP is not enabled").into_response();
    }
    if !check_and_consume_totp(&auth, &user, &body.totp_code) {
        return err(StatusCode::UNAUTHORIZED, "Invalid 2FA code").into_response();
    }
    if auth.db.disable_totp(&claims.sub).is_err() {
        return err(StatusCode::INTERNAL_SERVER_ERROR, "Failed to disable TOTP").into_response();
    }
    log::info!(
        "TOTP disabled for user '{}' ({}); sessions revoked via token_version bump",
        user.username,
        user.id
    );
    Json(serde_json::json!({
        "success": true,
        "totp_enabled": false,
        "sessions_revoked": true
    }))
    .into_response()
}

pub async fn providers_handler(State(auth): State<SharedAuth>) -> impl IntoResponse {
    let list: Vec<ProviderInfo> = match &auth.oidc {
        Some(cfg) => vec![ProviderInfo {
            id: cfg.id.clone(),
            name: cfg.name.clone(),
        }],
        None => Vec::new(),
    };
    Json(list).into_response()
}

pub async fn oidc_login_handler(
    State(auth): State<SharedAuth>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let Some(cfg) = auth.oidc.as_ref() else {
        return err(
            StatusCode::NOT_FOUND,
            "OIDC is not configured (set ZORVIA_OIDC_ENABLED=1 and issuer/client/secret/redirect)",
        )
        .into_response();
    };
    if id != cfg.id && id != "default" {
        return err(StatusCode::NOT_FOUND, "Unknown OIDC provider").into_response();
    }

    let endpoints = match discover(cfg).await {
        Ok(e) => e,
        Err(e) => {
            log::error!("OIDC discovery failed: {e:#}");
            return err(StatusCode::BAD_GATEWAY, "OIDC discovery failed").into_response();
        }
    };

    let state = random_urlsafe(32);
    let nonce = random_urlsafe(32);
    let verifier = random_urlsafe(48);
    let challenge = pkce_challenge_s256(&verifier);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    {
        let mut pending = match auth.oidc_pending.lock() {
            Ok(p) => p,
            Err(_) => {
                return err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "OIDC state store poisoned",
                )
                .into_response()
            }
        };
        // Drop expired entries.
        pending.retain(|_, v| now.saturating_sub(v.created_unix) < OIDC_PENDING_TTL_SECS);
        pending.insert(
            state.clone(),
            PendingOidc {
                code_verifier: verifier,
                nonce: nonce.clone(),
                created_unix: now,
                token_endpoint: endpoints.token_endpoint.clone(),
                jwks_uri: endpoints.jwks_uri.clone(),
                issuer: endpoints.issuer.clone(),
            },
        );
    }

    let url = build_authorize_url(
        &endpoints.authorization_endpoint,
        cfg,
        &state,
        &nonce,
        &challenge,
    );
    // Bind the flow to this browser: the callback must present the same state
    // in a cookie, so a victim cannot be lured into finishing a flow an
    // attacker started (login CSRF). Lax so it rides the top-level redirect
    // back from the IdP.
    let mut resp = Json(serde_json::json!({ "url": url })).into_response();
    if let Ok(v) = axum::http::HeaderValue::from_str(&oidc_state_cookie(
        &state,
        OIDC_PENDING_TTL_SECS,
        &cfg.redirect_uri,
    )) {
        resp.headers_mut().insert(axum::http::header::SET_COOKIE, v);
    }
    resp
}

const OIDC_STATE_COOKIE: &str = "zorvia_oidc_state";

fn oidc_state_cookie(value: &str, max_age: u64, redirect_uri: &str) -> String {
    let secure = if redirect_uri.starts_with("https://") {
        "; Secure"
    } else {
        ""
    };
    format!(
        "{OIDC_STATE_COOKIE}={value}; Path=/api/v1/auth/oidc; Max-Age={max_age}; HttpOnly; SameSite=Lax{secure}"
    )
}

fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(axum::http::header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v)
}

#[derive(Debug, Deserialize)]
pub struct OidcCallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

pub async fn oidc_callback_handler(
    State(auth): State<SharedAuth>,
    headers: HeaderMap,
    Query(q): Query<OidcCallbackQuery>,
) -> impl IntoResponse {
    let Some(cfg) = auth.oidc.as_ref() else {
        return err(StatusCode::NOT_FOUND, "OIDC is not configured").into_response();
    };
    if let Some(e) = q.error.as_deref() {
        return err(
            StatusCode::BAD_REQUEST,
            &format!("OIDC provider error: {e}"),
        )
        .into_response();
    }
    let (Some(code), Some(state)) = (q.code.as_deref(), q.state.as_deref()) else {
        return err(StatusCode::BAD_REQUEST, "Missing code or state").into_response();
    };
    if cookie_value(&headers, OIDC_STATE_COOKIE) != Some(state) {
        return err(
            StatusCode::BAD_REQUEST,
            "OIDC sign-in was not started in this browser",
        )
        .into_response();
    }

    let pending = {
        let mut map = match auth.oidc_pending.lock() {
            Ok(p) => p,
            Err(_) => {
                return err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "OIDC state store poisoned",
                )
                .into_response()
            }
        };
        match map.remove(state) {
            Some(p) => p,
            None => {
                return err(StatusCode::BAD_REQUEST, "Invalid or expired OIDC state")
                    .into_response()
            }
        }
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if now.saturating_sub(pending.created_unix) > OIDC_PENDING_TTL_SECS {
        return err(StatusCode::BAD_REQUEST, "OIDC state expired").into_response();
    }

    let id_token =
        match exchange_code(cfg, &pending.token_endpoint, code, &pending.code_verifier).await {
            Ok(t) => t,
            Err(e) => {
                log::error!("OIDC token exchange failed: {e:#}");
                return err(StatusCode::BAD_GATEWAY, "OIDC token exchange failed").into_response();
            }
        };

    let claims = match verify_id_token(
        &id_token,
        cfg,
        &pending.issuer,
        &pending.nonce,
        &pending.jwks_uri,
    )
    .await
    {
        Ok(c) => c,
        Err(e) => {
            log::error!("OIDC id_token verification failed: {e:#}");
            return err(
                StatusCode::UNAUTHORIZED,
                "OIDC id_token verification failed",
            )
            .into_response();
        }
    };

    let display = display_username(&claims);
    // The IdP's groups decide the role and namespaces at every login (when configured).
    let decision = auth
        .oidc_groups
        .as_ref()
        .map(|p| p.decide(&super::groups::groups_from_claims(&claims.extra, &p.claim)));
    let (initial_role, sync) = match decision {
        None => (Role::User, None),
        Some(super::groups::Decision::Allow { role, namespaces }) => {
            (role.clone(), Some((role, namespaces)))
        }
        Some(super::groups::Decision::Deny) => {
            // No matching group: no access, and any session this user holds is revoked.
            if let Ok(Some(existing)) = auth.db.get_by_username(&format!("oidc:{}", claims.sub)) {
                let _ = auth.db.bump_token_version(&existing.id);
            }
            log::warn!(
                "OIDC login denied: no group of subject {} maps to a role",
                claims.sub
            );
            return err(
                StatusCode::FORBIDDEN,
                "Your identity provider groups do not grant access to Zorvia",
            )
            .into_response();
        }
    };
    let user = match auth.db.upsert_oidc_user(&claims.sub, initial_role) {
        Ok(u) => u,
        Err(e) => {
            log::error!("OIDC JIT user upsert failed: {e:#}");
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to provision user",
            )
            .into_response();
        }
    };
    let user = match sync {
        Some((role, namespaces)) => match sync_oidc_user(&auth.db, user, role, namespaces) {
            Ok(u) => u,
            Err(e) => {
                log::error!("OIDC group sync failed: {e:#}");
                return err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to apply group mapping",
                )
                .into_response();
            }
        },
        None => user,
    };
    if !user.enabled {
        return err(StatusCode::FORBIDDEN, "This account has been disabled").into_response();
    }

    let token = match auth
        .jwt
        .generate(&user.id, &display, user.role.clone(), user.token_version)
    {
        Ok(t) => t,
        Err(e) => {
            log::error!("OIDC JWT mint failed: {e:#}");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "Failed to mint session")
                .into_response();
        }
    };

    // Fragment, not query: it is never sent to a server, logged, or put in a
    // Referer header.
    let redirect = format!("/sign-in#oidc_token={}", urlencoding::encode(&token));
    let mut resp = Redirect::temporary(&redirect).into_response();
    if let Ok(v) = axum::http::HeaderValue::from_str(&oidc_state_cookie("", 0, &cfg.redirect_uri)) {
        resp.headers_mut().insert(axum::http::header::SET_COOKIE, v);
    }
    resp
}

// ── API tokens ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateApiTokenRequest {
    pub name: String,
    pub role: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    pub expires_at: Option<String>,
    /// Confine the token to these namespaces (omit for unrestricted).
    #[serde(default)]
    pub namespaces: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
struct ApiTokenSummary {
    id: String,
    name: String,
    role: String,
    scopes: Vec<String>,
    expires_at: Option<String>,
    created_by: Option<String>,
    created: String,
    last_used: Option<String>,
    revoked: bool,
    namespaces: Option<Vec<String>>,
}

impl From<&ApiTokenRecord> for ApiTokenSummary {
    fn from(t: &ApiTokenRecord) -> Self {
        Self {
            id: t.id.clone(),
            name: t.name.clone(),
            role: role_str(&t.role).to_string(),
            scopes: t.scopes.clone(),
            expires_at: t.expires_at.clone(),
            created_by: t.created_by.clone(),
            created: t.created.clone(),
            last_used: t.last_used.clone(),
            revoked: t.revoked,
            namespaces: t.namespaces.clone(),
        }
    }
}

pub async fn list_api_tokens_handler(
    State(auth): State<SharedAuth>,
    _headers: HeaderMap,
) -> impl IntoResponse {
    match auth.db.list_api_tokens() {
        Ok(tokens) => {
            Json(tokens.iter().map(ApiTokenSummary::from).collect::<Vec<_>>()).into_response()
        }
        Err(_) => err(StatusCode::INTERNAL_SERVER_ERROR, "Failed to list tokens").into_response(),
    }
}

pub async fn create_api_token_handler(
    State(auth): State<SharedAuth>,
    headers: HeaderMap,
    Json(body): Json<CreateApiTokenRequest>,
) -> impl IntoResponse {
    if body.name.trim().is_empty() {
        return err(StatusCode::BAD_REQUEST, "Token name required").into_response();
    }
    let Some(role) = parse_role(&body.role) else {
        return err(StatusCode::BAD_REQUEST, "Invalid role").into_response();
    };
    if let Some(exp) = body.expires_at.as_deref() {
        if chrono::DateTime::parse_from_rfc3339(exp).is_err() {
            return err(
                StatusCode::BAD_REQUEST,
                "expires_at must be an RFC 3339 timestamp (e.g. 2027-01-31T00:00:00Z)",
            )
            .into_response();
        }
    }
    if let Some(list) = &body.namespaces {
        if list.len() > 64 || list.iter().any(|n| !valid_ns_name(n)) {
            return err(StatusCode::BAD_REQUEST, "Invalid namespace list").into_response();
        }
    }
    let created_by = bearer_token(&headers)
        .and_then(|t| auth.validate_bearer(&t))
        .map(|c| c.username);
    match auth.db.create_api_token(
        body.name.trim(),
        role,
        body.scopes,
        body.expires_at.as_deref(),
        created_by.as_deref(),
    ) {
        Ok((mut rec, plaintext)) => {
            if let Some(list) = body.namespaces.as_deref() {
                if auth
                    .db
                    .set_api_token_namespaces(&rec.id, Some(list))
                    .is_err()
                {
                    // Never hand out a token that was meant to be restricted but is not.
                    let _ = auth.db.revoke_api_token(&rec.id);
                    return err(StatusCode::INTERNAL_SERVER_ERROR, "Failed to create token")
                        .into_response();
                }
                rec.namespaces = Some(list.to_vec());
            }
            let mut summary = serde_json::to_value(ApiTokenSummary::from(&rec))
                .unwrap_or_else(|_| serde_json::json!({}));
            if let Some(obj) = summary.as_object_mut() {
                obj.insert("token".into(), serde_json::json!(plaintext));
            }
            (StatusCode::CREATED, Json(summary)).into_response()
        }
        Err(_) => err(StatusCode::INTERNAL_SERVER_ERROR, "Failed to create token").into_response(),
    }
}

pub async fn revoke_api_token_handler(
    State(auth): State<SharedAuth>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match auth.db.revoke_api_token(&id) {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => err(StatusCode::NOT_FOUND, "Token not found").into_response(),
        Err(_) => err(StatusCode::INTERNAL_SERVER_ERROR, "Failed to revoke token").into_response(),
    }
}

pub async fn delete_api_token_handler(
    State(auth): State<SharedAuth>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match auth.db.delete_api_token(&id) {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => err(StatusCode::NOT_FOUND, "Token not found").into_response(),
        Err(_) => err(StatusCode::INTERNAL_SERVER_ERROR, "Failed to delete token").into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub old_password: String,
    pub new_password: String,
}

/// `POST /v1/auth/password`: change your own password. Bumps `token_version`,
/// which revokes every session including this one; log in again afterwards.
pub async fn change_password_handler(
    State(auth): State<SharedAuth>,
    headers: HeaderMap,
    Json(body): Json<ChangePasswordRequest>,
) -> impl IntoResponse {
    let Some(claims) = bearer_token(&headers).and_then(|t| auth.validate_bearer(&t)) else {
        return err(StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    };
    let Ok(Some(user)) = auth.db.get_by_id(&claims.sub) else {
        return err(
            StatusCode::BAD_REQUEST,
            "This identity has no local password",
        )
        .into_response();
    };
    if !verify_password_off_runtime(user.password_hash.clone(), body.old_password.clone())
        .await
        .unwrap_or(false)
    {
        return err(StatusCode::UNAUTHORIZED, "Invalid password").into_response();
    }
    if body.new_password.len() < 8 || body.new_password.len() > 72 {
        return err(
            StatusCode::BAD_REQUEST,
            "Password must be 8 to 72 characters",
        )
        .into_response();
    }
    if auth
        .db
        .update_password(&user.id, &body.new_password)
        .is_err()
    {
        return err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to update password",
        )
        .into_response();
    }
    Json(serde_json::json!({ "success": true, "sessions_revoked": true })).into_response()
}

/// Issue time in seconds, used as the `tv` of stateless PAM sessions.
fn pam_now() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as u32)
        .unwrap_or(0)
}

/// `POST /v1/auth/logout`: revoke every session of the caller (token_version
/// bump). PAM sessions are revoked by a per-user not-before time.
pub async fn logout_handler(
    State(auth): State<SharedAuth>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let Some(claims) = bearer_token(&headers).and_then(|t| auth.validate_bearer(&t)) else {
        return err(StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    };
    let revoked = match claims.sub.strip_prefix("pam:") {
        // Sessions issued up to and including this second are revoked.
        Some(name) => auth
            .db
            .revoke_pam_sessions(name, pam_now().saturating_add(1))
            .is_ok(),
        None => auth.db.bump_token_version(&claims.sub).is_ok(),
    };
    Json(serde_json::json!({ "success": true, "sessions_revoked": revoked })).into_response()
}

#[cfg(test)]
mod oidc_group_sync_tests {
    use super::*;

    #[test]
    fn group_changes_update_role_and_namespaces_and_revoke_old_sessions() {
        let db = UserDb::open(":memory:").unwrap();
        let u = db.upsert_oidc_user("sub-1", Role::User).unwrap();
        let tv0 = u.token_version;
        // Nothing changed: no revocation.
        let u = sync_oidc_user(&db, u, Role::User, None).unwrap();
        assert_eq!(u.token_version, tv0);
        // Narrowed to a namespace: allow-list stored, sessions revoked.
        let u = sync_oidc_user(&db, u, Role::User, Some(vec!["team-a".into()])).unwrap();
        assert_eq!(
            db.get_namespaces(&u.id).unwrap(),
            Some(vec!["team-a".to_string()])
        );
        assert!(u.token_version > tv0);
        // Promoted to admin: role changes, the allow-list is dropped (admins are unrestricted).
        let tv1 = u.token_version;
        let u = sync_oidc_user(&db, u, Role::Admin, Some(vec!["team-a".into()])).unwrap();
        assert_eq!(u.role, Role::Admin);
        assert_eq!(db.get_namespaces(&u.id).unwrap(), None);
        assert!(u.token_version > tv1);
        // Demoted again.
        let tv2 = u.token_version;
        let u = sync_oidc_user(&db, u, Role::Viewer, None).unwrap();
        assert_eq!(u.role, Role::Viewer);
        assert!(u.token_version > tv2);
    }
}

#[cfg(test)]
mod totp_at_rest_tests {
    use super::*;
    use crate::keys::{self, KeyProvider};
    use std::collections::HashMap;

    #[test]
    fn totp_secrets_are_sealed_at_rest_legacy_rows_migrate_and_copies_fail_closed() {
        let db = UserDb::open(":memory:").unwrap();
        let a = db.create_user("alice", "Password-123", Role::User).unwrap();
        let b = db.create_user("bob", "Password-123", Role::User).unwrap();
        // A legacy row, written before any provider existed.
        db.set_totp(&a.id, "JBSWY3DPEHPK3PXP", true).unwrap();
        if keys::provider().is_none() {
            assert_eq!(db.raw_totp(&a.id).0.as_deref(), Some("JBSWY3DPEHPK3PXP"));
        }
        let mut k = HashMap::new();
        k.insert("k1".to_string(), [5u8; 32]);
        keys::set_provider_for_tests(KeyProvider::local("k1", k).unwrap());
        // The migration seals it; the API still returns the plaintext secret.
        db.seal_existing_totp().unwrap();
        let raw = db.raw_totp(&a.id).0.unwrap();
        assert!(
            keys::is_sealed(&raw) && !raw.contains("JBSWY3DPEHPK3PXP"),
            "{raw}"
        );
        assert_eq!(
            db.get_by_id(&a.id).unwrap().unwrap().totp_secret.as_deref(),
            Some("JBSWY3DPEHPK3PXP")
        );
        assert_eq!(db.seal_existing_totp().unwrap(), 0, "idempotent");
        // New writes are sealed too, including the pending re-enrolment secret.
        db.set_totp_pending(&a.id, Some("PENDINGSECRET234"))
            .unwrap();
        let pend = db.raw_totp(&a.id).1.unwrap();
        assert!(keys::is_sealed(&pend) && !pend.contains("PENDINGSECRET234"));
        assert_eq!(
            db.get_totp_pending(&a.id).unwrap().as_deref(),
            Some("PENDINGSECRET234")
        );
        db.commit_totp_pending(&a.id, "PENDINGSECRET234").unwrap();
        assert!(keys::is_sealed(&db.raw_totp(&a.id).0.unwrap()));
        // Alice's ciphertext copied onto Bob's row does not become Bob's secret.
        db.overwrite_raw_totp(&b.id, &db.raw_totp(&a.id).0.unwrap());
        assert_eq!(db.get_by_id(&b.id).unwrap().unwrap().totp_secret, None);
        // The admin reset clears everything and revokes sessions.
        let tv = db.get_by_id(&a.id).unwrap().unwrap().token_version;
        db.disable_totp(&a.id).unwrap();
        let after = db.get_by_id(&a.id).unwrap().unwrap();
        assert!(!after.totp_enabled && after.totp_secret.is_none() && after.token_version > tv);
        assert_eq!(db.raw_totp(&a.id), (None, None));
    }
}

#[cfg(test)]
mod token_namespace_tests {
    use super::*;

    #[test]
    fn token_namespaces_roundtrip_and_a_corrupt_list_restricts() {
        let db = UserDb::open(":memory:").unwrap();
        let (rec, plain) = db
            .create_api_token("ci", Role::User, vec![], None, None)
            .unwrap();
        assert_eq!(
            db.lookup_api_token(&plain).unwrap().unwrap().namespaces,
            None
        );
        db.set_api_token_namespaces(&rec.id, Some(&["team-a".to_string()]))
            .unwrap();
        assert_eq!(
            db.lookup_api_token(&plain).unwrap().unwrap().namespaces,
            Some(vec!["team-a".to_string()])
        );
        // Listing shows it too.
        assert!(db
            .list_api_tokens()
            .unwrap()
            .iter()
            .any(|t| t.namespaces.as_deref() == Some(&["team-a".to_string()][..])));
        db.set_api_token_namespaces(&rec.id, None).unwrap();
        assert_eq!(
            db.lookup_api_token(&plain).unwrap().unwrap().namespaces,
            None
        );
    }

    #[test]
    fn an_unreadable_stored_list_means_no_namespaces_not_all() {
        let db = UserDb::open(":memory:").unwrap();
        let (rec, plain) = db
            .create_api_token("ci", Role::User, vec![], None, None)
            .unwrap();
        db.set_api_token_namespaces(&rec.id, Some(&[])).unwrap();
        assert_eq!(
            db.lookup_api_token(&plain).unwrap().unwrap().namespaces,
            Some(vec![]),
            "an empty list confines the token to nothing"
        );
    }
}

#[cfg(test)]
mod bootstrap_password_tests {
    use super::*;

    #[test]
    fn bootstrap_password_goes_to_an_owner_only_file() {
        let dir = std::env::temp_dir().join(format!("zorvia-boot-{}", std::process::id()));
        let path = write_bootstrap_password(&dir, "admin", "Zv-secret").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("user: admin") && text.contains("password: Zv-secret"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[cfg(test)]
mod audit_low_tests {
    use super::*;

    #[test]
    fn a_totp_step_is_accepted_once_and_only_forward() {
        let db = UserDb::open(":memory:").unwrap();
        let u = db.create_user("dave", "Password-123", Role::User).unwrap();
        assert!(db.consume_totp_step(&u.id, 100).unwrap());
        assert!(
            !db.consume_totp_step(&u.id, 100).unwrap(),
            "replay of the same step"
        );
        assert!(!db.consume_totp_step(&u.id, 99).unwrap(), "older step");
        assert!(db.consume_totp_step(&u.id, 101).unwrap());
    }

    #[test]
    fn unparseable_or_past_api_token_expiry_fails_closed() {
        let db = UserDb::open(":memory:").unwrap();
        let (_, good) = db
            .create_api_token(
                "ok",
                Role::Viewer,
                vec![],
                Some("2999-01-01T00:00:00Z"),
                None,
            )
            .unwrap();
        assert!(db.lookup_api_token(&good).unwrap().is_some());
        let (_, past) = db
            .create_api_token(
                "old",
                Role::Viewer,
                vec![],
                Some("2001-01-01T00:00:00Z"),
                None,
            )
            .unwrap();
        assert!(db.lookup_api_token(&past).unwrap().is_none());
        // A malformed value can only get in through direct DB edits; it must not mean "forever".
        let (_, junk) = db
            .create_api_token("junk", Role::Viewer, vec![], Some("next tuesday"), None)
            .unwrap();
        assert!(db.lookup_api_token(&junk).unwrap().is_none());
        let (_, none) = db
            .create_api_token("forever", Role::Viewer, vec![], None, None)
            .unwrap();
        assert!(db.lookup_api_token(&none).unwrap().is_some());
    }
}

#[cfg(test)]
mod totp_pending_tests {
    use super::*;

    #[test]
    fn pending_secret_leaves_current_one_active_until_committed() {
        let db = UserDb::open(":memory:").unwrap();
        let u = db.create_user("carol", "Password-123", Role::User).unwrap();
        db.set_totp(&u.id, "OLDSECRET", true).unwrap();
        db.set_totp_pending(&u.id, Some("NEWSECRET")).unwrap();
        let mid = db.get_by_id(&u.id).unwrap().unwrap();
        assert!(
            mid.totp_enabled,
            "2FA stays on while a replacement is pending"
        );
        assert_eq!(mid.totp_secret.as_deref(), Some("OLDSECRET"));
        let tv = mid.token_version;
        db.commit_totp_pending(&u.id, "NEWSECRET").unwrap();
        let after = db.get_by_id(&u.id).unwrap().unwrap();
        assert!(after.totp_enabled);
        assert_eq!(after.totp_secret.as_deref(), Some("NEWSECRET"));
        assert_eq!(after.token_version, tv + 1, "commit revokes sessions");
        assert_eq!(db.get_totp_pending(&u.id).unwrap(), None);
    }
}

#[cfg(test)]
mod pam_revocation_tests {
    use super::*;

    #[test]
    fn pam_logout_revokes_earlier_sessions_only() {
        let db = UserDb::open(":memory:").unwrap();
        assert_eq!(db.pam_not_before("bob").unwrap(), 0);
        db.revoke_pam_sessions("bob", 1000).unwrap();
        assert_eq!(db.pam_not_before("bob").unwrap(), 1000);
        assert_eq!(db.pam_not_before("alice").unwrap(), 0);
    }
}

#[cfg(test)]
mod oidc_cookie_tests {
    use super::*;

    #[test]
    fn cookie_is_read_back_and_scoped() {
        let c = oidc_state_cookie("abc", 600, "https://z.example/cb");
        assert!(c.contains("HttpOnly") && c.contains("SameSite=Lax") && c.contains("Secure"));
        assert!(!oidc_state_cookie("abc", 600, "http://lab/cb").contains("Secure"));
        let mut h = HeaderMap::new();
        h.insert(
            axum::http::header::COOKIE,
            "a=1; zorvia_oidc_state=abc; b=2".parse().unwrap(),
        );
        assert_eq!(cookie_value(&h, OIDC_STATE_COOKIE), Some("abc"));
        assert_eq!(cookie_value(&h, "missing"), None);
    }
}

#[derive(Debug, Deserialize)]
pub struct SetNamespacesRequest {
    /// `null` removes the restriction.
    pub namespaces: Option<Vec<String>>,
}

fn valid_ns_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 63
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !s.starts_with('-')
        && !s.ends_with('-')
}

/// `PUT /v1/users/{id}/namespaces` (users.admin). Admin accounts are never
/// restricted, so setting a list on one is refused rather than silently ignored.
pub async fn set_namespaces_handler(
    State(auth): State<SharedAuth>,
    Path(id): Path<String>,
    Json(body): Json<SetNamespacesRequest>,
) -> impl IntoResponse {
    let Ok(Some(user)) = auth.db.get_by_id(&id) else {
        return err(StatusCode::NOT_FOUND, "User not found").into_response();
    };
    if let Some(list) = &body.namespaces {
        if matches!(user.role, Role::Admin) {
            return err(
                StatusCode::BAD_REQUEST,
                "Admin accounts cannot be namespace-restricted",
            )
            .into_response();
        }
        if list.len() > 64 || list.iter().any(|n| !valid_ns_name(n)) {
            return err(StatusCode::BAD_REQUEST, "Invalid namespace list").into_response();
        }
    }
    if auth
        .db
        .set_namespaces(&id, body.namespaces.as_deref())
        .is_err()
    {
        return err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to save namespaces",
        )
        .into_response();
    }
    Json(serde_json::json!({ "id": id, "namespaces": body.namespaces })).into_response()
}

/// `DELETE /v1/users/{id}/totp` (users.admin): remove a user's second factor and revoke
/// their sessions. The recovery path for a lost authenticator or a lost encryption key.
pub async fn reset_totp_handler(
    State(auth): State<SharedAuth>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match auth.db.get_by_id(&id) {
        Ok(Some(_)) => {}
        Ok(None) => return err(StatusCode::NOT_FOUND, "User not found").into_response(),
        Err(_) => return err(StatusCode::INTERNAL_SERVER_ERROR, "Lookup failed").into_response(),
    }
    if auth.db.disable_totp(&id).is_err() {
        return err(StatusCode::INTERNAL_SERVER_ERROR, "Failed to reset 2FA").into_response();
    }
    log::warn!("2FA reset for user {id} by an administrator; sessions revoked");
    Json(serde_json::json!({ "id": id, "totp_enabled": false, "sessions_revoked": true }))
        .into_response()
}

pub async fn get_namespaces_handler(
    State(auth): State<SharedAuth>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match auth.db.get_namespaces(&id) {
        Ok(n) => Json(serde_json::json!({ "id": id, "namespaces": n })).into_response(),
        Err(_) => err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to read namespaces",
        )
        .into_response(),
    }
}

#[cfg(test)]
mod verify_off_runtime_tests {
    use super::verify_password_off_runtime;

    #[tokio::test]
    async fn accepts_right_and_rejects_wrong_password() {
        let hash = bcrypt::hash("correct horse", 4).unwrap();
        assert!(
            verify_password_off_runtime(hash.clone(), "correct horse".into())
                .await
                .unwrap()
        );
        assert!(!verify_password_off_runtime(hash, "wrong".into())
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn malformed_hash_is_an_error_not_a_panic() {
        assert!(
            verify_password_off_runtime("not-a-bcrypt-hash".into(), "x".into())
                .await
                .is_err()
        );
    }
}
