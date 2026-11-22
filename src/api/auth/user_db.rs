// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

use crate::store::{Backend, Row};
use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

use super::jwt::Role;

pub struct User {
    pub id: String,
    pub username: String,
    pub password_hash: String,
    pub role: Role,
    pub totp_secret: Option<String>,
    pub totp_enabled: bool,
    pub enabled: bool,
    pub token_version: u32,
    pub created: String,
    pub last_login: Option<String>,
}

impl User {
    pub fn verify_password(&self, password: &str) -> Result<bool> {
        Ok(bcrypt::verify(password, &self.password_hash)?)
    }
}

#[derive(Debug, Clone)]
pub struct ApiTokenRecord {
    pub id: String,
    pub name: String,
    pub role: Role,
    pub scopes: Vec<String>,
    pub expires_at: Option<String>,
    pub created_by: Option<String>,
    pub created: String,
    pub last_used: Option<String>,
    pub revoked: bool,
    /// `Some` confines the token to these namespaces (see `tenancy`); `None` is unrestricted.
    pub namespaces: Option<Vec<String>>,
}

const USER_COLS: &str = "id, username, password_hash, role, totp_secret, totp_enabled, enabled, created, last_login, COALESCE(token_version, 0)";
const TOKEN_COLS: &str =
    "id, name, role, scopes, expires_at, created_by, created, last_used, revoked, namespaces";

/// Users, API tokens and PAM revocations. Runs on SQLite (`ZORVIA_AUTH_DB`) or, with
/// `ZORVIA_DATABASE_URL` set, on PostgreSQL shared by every API replica.
pub struct UserDb {
    db: Backend,
}

/// Schema for PostgreSQL: the SQLite tables with every integer a BIGINT. The advisory
/// lock keeps replicas that start together from racing on `CREATE TABLE`.
const PG_SCHEMA: &str = "
SELECT pg_advisory_xact_lock(727272001);
CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    username TEXT UNIQUE NOT NULL,
    password_hash TEXT NOT NULL,
    role TEXT NOT NULL,
    totp_secret TEXT,
    totp_enabled BIGINT NOT NULL DEFAULT 0,
    created TEXT NOT NULL,
    enabled BIGINT NOT NULL DEFAULT 1,
    last_login TEXT,
    token_version BIGINT NOT NULL DEFAULT 0,
    totp_last_step BIGINT,
    totp_pending TEXT,
    namespaces TEXT
);
CREATE TABLE IF NOT EXISTS api_tokens (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    token_hash TEXT UNIQUE NOT NULL,
    role TEXT NOT NULL,
    scopes TEXT NOT NULL DEFAULT '[]',
    expires_at TEXT,
    created_by TEXT,
    created TEXT NOT NULL,
    last_used TEXT,
    revoked BIGINT NOT NULL DEFAULT 0,
    namespaces TEXT
);
CREATE TABLE IF NOT EXISTS pam_revocations (username TEXT PRIMARY KEY, not_before BIGINT NOT NULL);
CREATE TABLE IF NOT EXISTS login_failures (username TEXT PRIMARY KEY, failures BIGINT NOT NULL, last_failure BIGINT NOT NULL);
";

impl UserDb {
    pub fn open(path: &str) -> Result<Self> {
        let conn = if path == ":memory:" {
            rusqlite::Connection::open_in_memory()?
        } else {
            if let Some(parent) = std::path::Path::new(path).parent() {
                std::fs::create_dir_all(parent).ok();
            }
            rusqlite::Connection::open(path)?
        };
        let db = Backend::sqlite(conn);
        db.batch(
            "CREATE TABLE IF NOT EXISTS users (
                id TEXT PRIMARY KEY,
                username TEXT UNIQUE NOT NULL,
                password_hash TEXT NOT NULL,
                role TEXT NOT NULL,
                totp_secret TEXT,
                totp_enabled INTEGER NOT NULL DEFAULT 0,
                created TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS api_tokens (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                token_hash TEXT UNIQUE NOT NULL,
                role TEXT NOT NULL,
                scopes TEXT NOT NULL DEFAULT '[]',
                expires_at TEXT,
                created_by TEXT,
                created TEXT NOT NULL,
                last_used TEXT,
                revoked INTEGER NOT NULL DEFAULT 0
            );",
        )?;
        // Columns added after the tables above shipped: ALTER TABLE ADD COLUMN on an
        // existing DB, since SQLite's IF NOT EXISTS only guards table creation.
        for alter in [
            "ALTER TABLE users ADD COLUMN enabled INTEGER NOT NULL DEFAULT 1;",
            "ALTER TABLE users ADD COLUMN last_login TEXT;",
            "ALTER TABLE users ADD COLUMN token_version INTEGER NOT NULL DEFAULT 0;",
            // API tokens: NULL = unrestricted, else a JSON array of allowed namespaces.
            "ALTER TABLE api_tokens ADD COLUMN namespaces TEXT;",
            // Last accepted TOTP time step, so a code cannot be replayed.
            "ALTER TABLE users ADD COLUMN totp_last_step INTEGER;",
            // Replacement TOTP secret awaiting its first valid code (re-enrolment).
            "ALTER TABLE users ADD COLUMN totp_pending TEXT;",
            // NULL = unrestricted; otherwise a JSON array of allowed namespaces.
            "ALTER TABLE users ADD COLUMN namespaces TEXT;",
        ] {
            let _ = db.batch(alter);
        }
        // PAM sessions have no users row; logout records a not-before time here.
        let _ = db.batch(
            "CREATE TABLE IF NOT EXISTS pam_revocations (username TEXT PRIMARY KEY, not_before INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS login_failures (username TEXT PRIMARY KEY, failures INTEGER NOT NULL, last_failure INTEGER NOT NULL);",
        );
        Ok(Self { db })
    }

    /// Open the store on a PostgreSQL server (`postgres://user:pass@host/db?sslmode=verify-full`).
    pub fn open_postgres(url: &str) -> Result<Self> {
        let db = Backend::postgres(url)?;
        db.batch(PG_SCHEMA)?;
        Ok(Self { db })
    }

    pub fn is_postgres(&self) -> bool {
        self.db.is_postgres()
    }

    /// Reject PAM sessions for `username` issued before `now`.
    pub fn revoke_pam_sessions(&self, username: &str, now: u32) -> Result<()> {
        self.db.exec(
            "INSERT INTO pam_revocations (username, not_before) VALUES (?1, ?2)
             ON CONFLICT(username) DO UPDATE SET not_before = excluded.not_before",
            &[username.into(), now.into()],
        )?;
        Ok(())
    }

    /// Shared failed-login counter (all replicas on PostgreSQL see one count). Blocked once
    /// `max` failures were recorded and the last one is less than `lockout_secs` old.
    pub fn login_blocked(
        &self,
        username: &str,
        now: i64,
        max: i64,
        lockout_secs: i64,
    ) -> Result<bool> {
        Ok(self
            .db
            .query_one(
                "SELECT failures, last_failure FROM login_failures WHERE username = ?1",
                &[username.into()],
            )?
            .is_some_and(|r| r.int(0) >= max && now - r.int(1) < lockout_secs))
    }

    /// Count one failed sign-in atomically; a count older than the lockout starts again at 1,
    /// and every further failure extends the lockout.
    pub fn login_record_failure(&self, username: &str, now: i64, lockout_secs: i64) -> Result<()> {
        self.db.exec(
            "INSERT INTO login_failures (username, failures, last_failure) VALUES (?1, 1, ?2)
             ON CONFLICT (username) DO UPDATE SET
               failures = CASE WHEN login_failures.last_failure + ?3 <= ?2 THEN 1 ELSE login_failures.failures + 1 END,
               last_failure = ?2",
            &[username.into(), now.into(), lockout_secs.into()],
        )?;
        // Keep the table small: forget names whose lockout has long passed.
        self.db.exec(
            "DELETE FROM login_failures WHERE last_failure + ?2 < ?1",
            &[now.into(), (lockout_secs * 2).into()],
        )?;
        Ok(())
    }

    pub fn login_clear(&self, username: &str) -> Result<()> {
        self.db.exec(
            "DELETE FROM login_failures WHERE username = ?1",
            &[username.into()],
        )?;
        Ok(())
    }

    pub fn pam_not_before(&self, username: &str) -> Result<u32> {
        let n = self
            .db
            .query_one(
                "SELECT not_before FROM pam_revocations WHERE username = ?1",
                &[username.into()],
            )?
            .map(|r| r.int(0))
            .unwrap_or(0);
        Ok(n as u32)
    }

    /// `None` = unrestricted.
    pub fn get_namespaces(&self, user_id: &str) -> Result<Option<Vec<String>>> {
        let raw = self
            .db
            .query_one(
                "SELECT namespaces FROM users WHERE id = ?1",
                &[user_id.into()],
            )?
            .and_then(|r| r.opt_text(0));
        // A value that fails to parse must restrict, not unrestrict.
        Ok(raw.map(|s| serde_json::from_str(&s).unwrap_or_default()))
    }

    pub fn set_namespaces(&self, user_id: &str, namespaces: Option<&[String]>) -> Result<()> {
        let raw = namespaces.map(serde_json::to_string).transpose()?;
        self.db.exec(
            "UPDATE users SET namespaces = ?1 WHERE id = ?2",
            &[raw.into(), user_id.into()],
        )?;
        Ok(())
    }

    /// Path of the auth database (`ZORVIA_AUTH_DB`, else the user data dir).
    pub fn env_path() -> String {
        std::env::var("ZORVIA_AUTH_DB").unwrap_or_else(|_| {
            dirs::data_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("zorvia")
                .join("auth.db")
                .to_string_lossy()
                .to_string()
        })
    }

    /// PostgreSQL when `ZORVIA_DATABASE_URL` is set, otherwise the SQLite file.
    pub fn from_env() -> Result<Self> {
        match crate::store::database_url() {
            Some(url) => {
                let db = Self::open_postgres(&url)?;
                log::info!("auth store: PostgreSQL");
                Ok(db)
            }
            None => Self::open(&Self::env_path()),
        }
    }

    pub fn count_users(&self) -> Result<usize> {
        let n = self
            .db
            .query_one("SELECT COUNT(*) FROM users", &[])?
            .map(|r| r.int(0))
            .unwrap_or(0);
        Ok(n as usize)
    }

    pub fn seed_admin(&self, username: &str, password: &str) -> Result<Option<User>> {
        if self.count_users()? > 0 {
            return Ok(None);
        }
        // Two replicas starting together both see an empty table; the unique username
        // lets exactly one insert win and the other quietly stand down.
        let user = match self.create_user(username, password, Role::Admin) {
            Ok(u) => u,
            Err(e) if is_unique_violation(&e) => return Ok(None),
            Err(e) => return Err(e),
        };
        log::info!("Seeded bootstrap admin user '{}'", username);
        Ok(Some(user))
    }

    pub fn create_user(&self, username: &str, password: &str, role: Role) -> Result<User> {
        let id = uuid::Uuid::new_v4().to_string();
        let hash = bcrypt::hash(password, bcrypt::DEFAULT_COST)?;
        let role_str = role_to_str(&role);
        let created = chrono::Utc::now().to_rfc3339();
        self.db.exec(
            "INSERT INTO users (id, username, password_hash, role, created, token_version) VALUES (?1,?2,?3,?4,?5,0)",
            &[
                id.clone().into(),
                username.into(),
                hash.clone().into(),
                role_str.into(),
                created.clone().into(),
            ],
        )?;
        Ok(User {
            id,
            username: username.to_string(),
            password_hash: hash,
            role,
            totp_secret: None,
            totp_enabled: false,
            enabled: true,
            token_version: 0,
            created,
            last_login: None,
        })
    }

    /// Find or create a local user for a verified OIDC subject.
    /// Username is stable (`oidc:<sub>`) so JIT users participate in token_version revocation.
    pub fn upsert_oidc_user(&self, subject: &str, role: Role) -> Result<User> {
        self.upsert_federated_user("oidc", subject, role)
    }

    /// Same, for any federated protocol; `prefix` (`oidc`, `saml`) namespaces the username so a
    /// subject from one IdP type can never map onto another's user.
    pub fn upsert_federated_user(&self, prefix: &str, subject: &str, role: Role) -> Result<User> {
        let username = format!("{prefix}:{subject}");
        if let Some(existing) = self.get_by_username(&username)? {
            return Ok(existing);
        }
        // Unusable random password — OIDC users authenticate via IdP only.
        let pw = format!("oidc-{}", uuid::Uuid::new_v4());
        match self.create_user(&username, &pw, role) {
            Ok(u) => Ok(u),
            // Another replica provisioned the same subject a moment ago.
            Err(e) if is_unique_violation(&e) => self
                .get_by_username(&username)?
                .ok_or_else(|| anyhow::anyhow!("OIDC user vanished after a duplicate insert")),
            Err(e) => Err(e),
        }
    }

    pub fn get_by_username(&self, username: &str) -> Result<Option<User>> {
        self.db
            .query_one(
                &format!("SELECT {USER_COLS} FROM users WHERE username = ?1"),
                &[username.into()],
            )?
            .map(|r| row_to_user(&r))
            .transpose()
    }

    pub fn get_by_id(&self, id: &str) -> Result<Option<User>> {
        self.db
            .query_one(
                &format!("SELECT {USER_COLS} FROM users WHERE id = ?1"),
                &[id.into()],
            )?
            .map(|r| row_to_user(&r))
            .transpose()
    }

    pub fn list_users(&self) -> Result<Vec<User>> {
        self.db
            .query(
                &format!("SELECT {USER_COLS} FROM users ORDER BY created ASC"),
                &[],
            )?
            .iter()
            .map(row_to_user)
            .collect()
    }

    pub fn enabled_admin_count(&self) -> Result<usize> {
        let n = self
            .db
            .query_one(
                "SELECT COUNT(*) FROM users WHERE role = 'admin' AND enabled = 1",
                &[],
            )?
            .map(|r| r.int(0))
            .unwrap_or(0);
        Ok(n as usize)
    }

    pub fn delete_user(&self, id: &str) -> Result<()> {
        self.db
            .exec("DELETE FROM users WHERE id = ?1", &[id.into()])
            .context("delete user")?;
        Ok(())
    }

    pub fn update_role(&self, id: &str, role: Role) -> Result<()> {
        self.db
            .exec(
                "UPDATE users SET role = ?1, token_version = token_version + 1 WHERE id = ?2",
                &[role_to_str(&role).into(), id.into()],
            )
            .context("update role")?;
        Ok(())
    }

    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        if enabled {
            self.db
                .exec("UPDATE users SET enabled = 1 WHERE id = ?1", &[id.into()])
                .context("set enabled")?;
        } else {
            self.db
                .exec(
                    "UPDATE users SET enabled = 0, token_version = token_version + 1 WHERE id = ?1",
                    &[id.into()],
                )
                .context("set enabled")?;
        }
        Ok(())
    }

    pub fn bump_token_version(&self, id: &str) -> Result<()> {
        self.db
            .exec(
                "UPDATE users SET token_version = token_version + 1 WHERE id = ?1",
                &[id.into()],
            )
            .context("bump token_version")?;
        Ok(())
    }

    pub fn update_password(&self, id: &str, password: &str) -> Result<()> {
        let hash = bcrypt::hash(password, bcrypt::DEFAULT_COST)?;
        self.db
            .exec(
                "UPDATE users SET password_hash = ?1, token_version = token_version + 1 WHERE id = ?2",
                &[hash.into(), id.into()],
            )
            .context("update password")?;
        Ok(())
    }

    pub fn update_last_login(&self, id: &str) -> Result<()> {
        self.db
            .exec(
                "UPDATE users SET last_login = ?1 WHERE id = ?2",
                &[chrono::Utc::now().to_rfc3339().into(), id.into()],
            )
            .context("update last_login")?;
        Ok(())
    }

    pub fn set_totp(&self, user_id: &str, secret: &str, enabled: bool) -> Result<()> {
        let stored = seal_for(user_id, secret)?;
        self.db
            .exec(
                "UPDATE users SET totp_secret = ?1, totp_enabled = ?2 WHERE id = ?3",
                &[stored.into(), enabled.into(), user_id.into()],
            )
            .context("update totp")?;
        Ok(())
    }

    /// Accept `step` only if it is newer than the last one accepted for this user. A single
    /// conditional UPDATE, so two replicas cannot both accept the same code.
    pub fn consume_totp_step(&self, user_id: &str, step: u64) -> Result<bool> {
        let n = self.db.exec(
            "UPDATE users SET totp_last_step = ?1
             WHERE id = ?2 AND (totp_last_step IS NULL OR totp_last_step < ?1)",
            &[step.into(), user_id.into()],
        )?;
        Ok(n == 1)
    }

    pub fn set_totp_pending(&self, user_id: &str, secret: Option<&str>) -> Result<()> {
        let stored = secret.map(|s| seal_for(user_id, s)).transpose()?;
        self.db.exec(
            "UPDATE users SET totp_pending = ?1 WHERE id = ?2",
            &[stored.into(), user_id.into()],
        )?;
        Ok(())
    }

    pub fn get_totp_pending(&self, user_id: &str) -> Result<Option<String>> {
        let raw = self
            .db
            .query_one(
                "SELECT totp_pending FROM users WHERE id = ?1",
                &[user_id.into()],
            )?
            .and_then(|r| r.opt_text(0));
        Ok(open_stored(user_id, raw))
    }

    /// Swap the pending secret in, keep 2FA on, and revoke existing sessions.
    pub fn commit_totp_pending(&self, user_id: &str, secret: &str) -> Result<()> {
        let stored = seal_for(user_id, secret)?;
        self.db.exec(
            "UPDATE users SET totp_secret = ?1, totp_enabled = 1, totp_pending = NULL, token_version = token_version + 1 WHERE id = ?2",
            &[stored.into(), user_id.into()],
        )?;
        Ok(())
    }

    /// With a key provider configured: seal every TOTP secret that is still plaintext
    /// (or sealed under a rotated-out key id) and return how many rows changed.
    /// Idempotent; run at startup. Without a provider it only reports sealed values
    /// that can no longer be read (a missing key), which would lock those users out.
    pub fn seal_existing_totp(&self) -> Result<usize> {
        let rows: Vec<(String, Option<String>, Option<String>)> = self
            .db
            .query(
                "SELECT id, totp_secret, totp_pending FROM users
                 WHERE totp_secret IS NOT NULL OR totp_pending IS NOT NULL",
                &[],
            )?
            .iter()
            .map(|r| Ok((r.text(0)?, r.opt_text(1), r.opt_text(2))))
            .collect::<Result<_>>()?;
        let Some(p) = crate::keys::provider() else {
            let sealed = rows
                .iter()
                .filter(|(_, a, b)| {
                    [a, b]
                        .iter()
                        .any(|v| v.as_deref().is_some_and(crate::keys::is_sealed))
                })
                .count();
            if sealed > 0 {
                log::error!(
                    "{sealed} user(s) have encrypted TOTP secrets but no key provider is configured: \
                     they cannot sign in with 2FA until the key is configured or an admin resets their 2FA"
                );
            }
            return Ok(0);
        };
        let mut changed = 0;
        for (id, secret, pending) in rows {
            let mut fix = |v: Option<String>| -> Result<Option<String>> {
                let Some(v) = v else { return Ok(None) };
                if !p.needs_seal(&v) {
                    return Ok(Some(v));
                }
                let plain = if crate::keys::is_sealed(&v) {
                    match p.open(&id, &v) {
                        Ok(p) => p,
                        Err(e) => {
                            log::error!("user {id}: cannot re-seal a TOTP secret: {e:#}");
                            return Ok(Some(v));
                        }
                    }
                } else {
                    v.clone()
                };
                changed += 1;
                Ok(Some(p.seal(&id, &plain)?))
            };
            let (s2, p2) = (fix(secret.clone())?, fix(pending.clone())?);
            if s2 != secret || p2 != pending {
                self.db.exec(
                    "UPDATE users SET totp_secret = ?1, totp_pending = ?2 WHERE id = ?3",
                    &[s2.into(), p2.into(), id.into()],
                )?;
            }
        }
        Ok(changed)
    }

    pub fn disable_totp(&self, user_id: &str) -> Result<()> {
        self.db.exec(
            "UPDATE users SET totp_secret = NULL, totp_pending = NULL, totp_enabled = 0, token_version = token_version + 1 WHERE id = ?1",
            &[user_id.into()],
        )?;
        Ok(())
    }

    // ── API tokens ───────────────────────────────────────────────

    pub fn hash_api_token(plaintext: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(plaintext.as_bytes());
        hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    /// Creates a token; returns `(record, plaintext)` — plaintext shown once.
    pub fn create_api_token(
        &self,
        name: &str,
        role: Role,
        scopes: Vec<String>,
        expires_at: Option<&str>,
        created_by: Option<&str>,
    ) -> Result<(ApiTokenRecord, String)> {
        let id = uuid::Uuid::new_v4().to_string();
        let plaintext = format!("zrv_{}", uuid::Uuid::new_v4());
        let token_hash = Self::hash_api_token(&plaintext);
        let created = chrono::Utc::now().to_rfc3339();
        let scopes_json = serde_json::to_string(&scopes)?;
        self.db.exec(
            "INSERT INTO api_tokens (id, name, token_hash, role, scopes, expires_at, created_by, created, revoked)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,0)",
            &[
                id.clone().into(),
                name.into(),
                token_hash.into(),
                role_to_str(&role).into(),
                scopes_json.into(),
                expires_at.into(),
                created_by.into(),
                created.clone().into(),
            ],
        )?;
        Ok((
            ApiTokenRecord {
                id,
                name: name.to_string(),
                role,
                scopes,
                expires_at: expires_at.map(|s| s.to_string()),
                created_by: created_by.map(|s| s.to_string()),
                created,
                last_used: None,
                revoked: false,
                namespaces: None,
            },
            plaintext,
        ))
    }

    pub fn lookup_api_token(&self, plaintext: &str) -> Result<Option<ApiTokenRecord>> {
        let hash = Self::hash_api_token(plaintext);
        let Some(row) = self.db.query_one(
            &format!("SELECT {TOKEN_COLS} FROM api_tokens WHERE token_hash = ?1"),
            &[hash.into()],
        )?
        else {
            return Ok(None);
        };
        let rec = row_to_api_token(&row)?;
        if rec.revoked {
            return Ok(None);
        }
        if let Some(ref exp) = rec.expires_at {
            // An expiry that cannot be read is treated as expired, never as "no expiry".
            match chrono::DateTime::parse_from_rfc3339(exp) {
                Ok(dt) if dt >= chrono::Utc::now() => {}
                _ => return Ok(None),
            }
        }
        Ok(Some(rec))
    }

    #[cfg(test)]
    pub(crate) fn raw_totp(&self, id: &str) -> (Option<String>, Option<String>) {
        let r = self
            .db
            .query_one(
                "SELECT totp_secret, totp_pending FROM users WHERE id = ?1",
                &[id.into()],
            )
            .unwrap()
            .unwrap();
        (r.opt_text(0), r.opt_text(1))
    }

    #[cfg(test)]
    pub(crate) fn overwrite_raw_totp(&self, id: &str, secret: &str) {
        self.db
            .exec(
                "UPDATE users SET totp_secret = ?1 WHERE id = ?2",
                &[secret.into(), id.into()],
            )
            .unwrap();
    }

    pub fn set_api_token_namespaces(&self, id: &str, namespaces: Option<&[String]>) -> Result<()> {
        let raw = namespaces.map(serde_json::to_string).transpose()?;
        self.db.exec(
            "UPDATE api_tokens SET namespaces = ?1 WHERE id = ?2",
            &[raw.into(), id.into()],
        )?;
        Ok(())
    }

    pub fn touch_api_token(&self, id: &str) -> Result<()> {
        self.db.exec(
            "UPDATE api_tokens SET last_used = ?1 WHERE id = ?2",
            &[chrono::Utc::now().to_rfc3339().into(), id.into()],
        )?;
        Ok(())
    }

    pub fn list_api_tokens(&self) -> Result<Vec<ApiTokenRecord>> {
        self.db
            .query(
                &format!("SELECT {TOKEN_COLS} FROM api_tokens ORDER BY created ASC"),
                &[],
            )?
            .iter()
            .map(row_to_api_token)
            .collect()
    }

    pub fn revoke_api_token(&self, id: &str) -> Result<bool> {
        let n = self.db.exec(
            "UPDATE api_tokens SET revoked = 1 WHERE id = ?1 AND revoked = 0",
            &[id.into()],
        )?;
        Ok(n > 0)
    }

    pub fn delete_api_token(&self, id: &str) -> Result<bool> {
        let n = self
            .db
            .exec("DELETE FROM api_tokens WHERE id = ?1", &[id.into()])?;
        Ok(n > 0)
    }

    /// Copy users, tokens and PAM revocations from a SQLite auth database into this
    /// (empty) store, keeping ids, password hashes and sealed TOTP secrets as they are.
    /// Used once when moving from SQLite to PostgreSQL; refuses a store that already has users.
    pub fn import_from_sqlite(&self, path: &str) -> Result<usize> {
        if self.count_users()? > 0 {
            bail!("the target store already has users; refusing to import over them");
        }
        let src = UserDb::open(path)?;
        let mut n = 0;
        for r in src.db.query("SELECT id, username, password_hash, role, totp_secret, totp_enabled, created, enabled, last_login, COALESCE(token_version,0), totp_last_step, totp_pending, namespaces FROM users", &[])? {
            self.db.exec(
                "INSERT INTO users (id, username, password_hash, role, totp_secret, totp_enabled, created, enabled, last_login, token_version, totp_last_step, totp_pending, namespaces)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
                &r.0,
            )?;
            n += 1;
        }
        for r in src.db.query("SELECT id, name, token_hash, role, scopes, expires_at, created_by, created, last_used, revoked, namespaces FROM api_tokens", &[])? {
            self.db.exec(
                "INSERT INTO api_tokens (id, name, token_hash, role, scopes, expires_at, created_by, created, last_used, revoked, namespaces)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                &r.0,
            )?;
        }
        for r in src
            .db
            .query("SELECT username, not_before FROM pam_revocations", &[])?
        {
            self.db.exec(
                "INSERT INTO pam_revocations (username, not_before) VALUES (?1, ?2)",
                &r.0,
            )?;
        }
        Ok(n)
    }
}

/// A duplicate-key error from either backend.
fn is_unique_violation(e: &anyhow::Error) -> bool {
    e.chain().any(|c| {
        let m = c.to_string();
        m.contains("UNIQUE constraint failed")
            || m.contains("duplicate key value")
            || c.downcast_ref::<tokio_postgres::Error>()
                .and_then(|pe| pe.code())
                .is_some_and(|code| *code == tokio_postgres::error::SqlState::UNIQUE_VIOLATION)
    })
}

fn role_to_str(role: &Role) -> &'static str {
    match role {
        Role::Admin => "admin",
        Role::User => "user",
        Role::Viewer => "viewer",
    }
}

fn parse_role_str(s: &str) -> Role {
    match s {
        "admin" => Role::Admin,
        "viewer" => Role::Viewer,
        _ => Role::User,
    }
}

/// Seal a TOTP secret for `user_id` when a key provider is configured (an error is
/// never swallowed into plaintext), else keep it as is.
fn seal_for(user_id: &str, plain: &str) -> Result<String> {
    match crate::keys::provider() {
        Some(p) => p.seal(user_id, plain),
        None => Ok(plain.to_string()),
    }
}

/// Read a stored TOTP value: plaintext as is, a sealed one opened with the provider.
/// An unreadable sealed value (no provider, wrong or missing key) is `None` and logged,
/// so verification fails closed instead of treating ciphertext as a secret.
fn open_stored(user_id: &str, stored: Option<String>) -> Option<String> {
    let s = stored?;
    if !crate::keys::is_sealed(&s) {
        return Some(s);
    }
    match crate::keys::provider() {
        Some(p) => match p.open(user_id, &s) {
            Ok(plain) => Some(plain),
            Err(e) => {
                log::error!("user {user_id}: cannot decrypt the TOTP secret ({e:#}); an admin can reset their 2FA");
                None
            }
        },
        None => {
            log::error!(
                "user {user_id}: the TOTP secret is encrypted but no key provider is configured"
            );
            None
        }
    }
}

fn row_to_user(row: &Row) -> Result<User> {
    let id = row.text(0)?;
    Ok(User {
        username: row.text(1)?,
        password_hash: row.text(2)?,
        role: parse_role_str(&row.text(3)?),
        totp_secret: open_stored(&id, row.opt_text(4)),
        totp_enabled: row.int(5) != 0,
        enabled: row.int(6) != 0,
        created: row.text(7)?,
        last_login: row.opt_text(8),
        token_version: row.int(9) as u32,
        id,
    })
}

fn row_to_api_token(row: &Row) -> Result<ApiTokenRecord> {
    let scopes: Vec<String> = serde_json::from_str(&row.text(3)?).unwrap_or_default();
    Ok(ApiTokenRecord {
        id: row.text(0)?,
        name: row.text(1)?,
        role: parse_role_str(&row.text(2)?),
        scopes,
        expires_at: row.opt_text(4),
        created_by: row.opt_text(5),
        created: row.text(6)?,
        last_used: row.opt_text(7),
        revoked: row.int(8) != 0,
        // An unreadable list must restrict, never unrestrict.
        namespaces: row
            .opt_text(9)
            .map(|s| serde_json::from_str(&s).unwrap_or_default()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_version_bumps_on_disable() {
        let db = UserDb::open(":memory:").unwrap();
        let u = db.create_user("alice", "password123", Role::User).unwrap();
        assert_eq!(u.token_version, 0);
        db.set_enabled(&u.id, false).unwrap();
        let u2 = db.get_by_id(&u.id).unwrap().unwrap();
        assert!(!u2.enabled);
        assert_eq!(u2.token_version, 1);
    }

    #[test]
    fn api_token_hash_is_lowercase_sha256_hex() {
        assert_eq!(
            UserDb::hash_api_token("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn api_token_roundtrip() {
        let db = UserDb::open(":memory:").unwrap();
        let (rec, plain) = db
            .create_api_token("ci", Role::User, vec!["vm.read".into()], None, None)
            .unwrap();
        let found = db.lookup_api_token(&plain).unwrap().unwrap();
        assert_eq!(found.id, rec.id);
        assert_eq!(found.role, Role::User);
        db.revoke_api_token(&rec.id).unwrap();
        assert!(db.lookup_api_token(&plain).unwrap().is_none());
    }

    /// The same behaviour on every backend. Run for SQLite always and for PostgreSQL when
    /// `ZORVIA_TEST_POSTGRES_URL` points at a throwaway database.
    fn conformance(db: &UserDb) {
        // Shared failed-login counter.
        let (max, lock) = (3, 100);
        assert!(!db.login_blocked("Mallory", 1000, max, lock).unwrap());
        for _ in 0..3 {
            db.login_record_failure("Mallory", 1000, lock).unwrap();
        }
        assert!(db.login_blocked("Mallory", 1050, max, lock).unwrap());
        assert!(!db.login_blocked("someone-else", 1050, max, lock).unwrap());
        assert!(
            !db.login_blocked("Mallory", 1100, max, lock).unwrap(),
            "lockout over"
        );
        db.login_record_failure("Mallory", 1101, lock).unwrap();
        assert!(
            !db.login_blocked("Mallory", 1102, max, lock).unwrap(),
            "an old count starts again at one"
        );
        db.login_record_failure("Mallory", 1102, lock).unwrap();
        db.login_record_failure("Mallory", 1103, lock).unwrap();
        assert!(db.login_blocked("Mallory", 1104, max, lock).unwrap());
        db.login_clear("Mallory").unwrap();
        assert!(!db.login_blocked("Mallory", 1104, max, lock).unwrap());

        // Users, uniqueness, roles, revocation counter.
        let a = db.create_user("alice", "password123", Role::User).unwrap();
        assert!(
            db.create_user("alice", "x-password", Role::User).is_err(),
            "duplicate username"
        );
        assert!(is_unique_violation(
            &db.create_user("alice", "x-password", Role::User)
                .err()
                .unwrap()
        ));
        assert_eq!(db.get_by_username("alice").unwrap().unwrap().id, a.id);
        assert!(db.get_by_username("nobody").unwrap().is_none());
        assert_eq!(db.count_users().unwrap(), 1);
        db.update_role(&a.id, Role::Admin).unwrap();
        let u = db.get_by_id(&a.id).unwrap().unwrap();
        assert_eq!((u.role, u.token_version), (Role::Admin, 1));
        assert_eq!(db.enabled_admin_count().unwrap(), 1);
        db.set_enabled(&a.id, false).unwrap();
        assert_eq!(db.enabled_admin_count().unwrap(), 0);
        db.update_password(&a.id, "new-password-1").unwrap();
        let u = db.get_by_id(&a.id).unwrap().unwrap();
        assert!(!u.enabled && u.token_version == 3 && u.verify_password("new-password-1").unwrap());
        db.update_last_login(&a.id).unwrap();
        assert!(db.get_by_id(&a.id).unwrap().unwrap().last_login.is_some());
        let b = db.create_user("bob", "password123", Role::Viewer).unwrap();
        assert_eq!(
            db.list_users()
                .unwrap()
                .iter()
                .map(|u| u.username.as_str())
                .collect::<Vec<_>>(),
            ["alice", "bob"]
        );

        // Namespaces: NULL means unrestricted, a list restricts, an empty list is nothing.
        assert_eq!(db.get_namespaces(&b.id).unwrap(), None);
        db.set_namespaces(&b.id, Some(&["team-a".to_string()]))
            .unwrap();
        assert_eq!(
            db.get_namespaces(&b.id).unwrap(),
            Some(vec!["team-a".to_string()])
        );
        db.set_namespaces(&b.id, Some(&[])).unwrap();
        assert_eq!(db.get_namespaces(&b.id).unwrap(), Some(vec![]));
        db.set_namespaces(&b.id, None).unwrap();
        assert_eq!(db.get_namespaces(&b.id).unwrap(), None);

        // TOTP: a time step is accepted once, only moving forward; pending then commit.
        db.set_totp(&b.id, "JBSWY3DPEHPK3PXP", true).unwrap();
        assert_eq!(
            db.get_by_id(&b.id).unwrap().unwrap().totp_secret.as_deref(),
            Some("JBSWY3DPEHPK3PXP")
        );
        assert!(db.consume_totp_step(&b.id, 100).unwrap());
        assert!(!db.consume_totp_step(&b.id, 100).unwrap());
        assert!(!db.consume_totp_step(&b.id, 99).unwrap());
        assert!(db.consume_totp_step(&b.id, 101).unwrap());
        db.set_totp_pending(&b.id, Some("PENDINGSECRET234"))
            .unwrap();
        assert_eq!(
            db.get_totp_pending(&b.id).unwrap().as_deref(),
            Some("PENDINGSECRET234")
        );
        let tv = db.get_by_id(&b.id).unwrap().unwrap().token_version;
        db.commit_totp_pending(&b.id, "PENDINGSECRET234").unwrap();
        let ub = db.get_by_id(&b.id).unwrap().unwrap();
        assert_eq!(ub.totp_secret.as_deref(), Some("PENDINGSECRET234"));
        assert!(ub.token_version > tv);
        assert_eq!(db.get_totp_pending(&b.id).unwrap(), None);
        db.disable_totp(&b.id).unwrap();
        assert!(!db.get_by_id(&b.id).unwrap().unwrap().totp_enabled);

        // PAM revocations.
        assert_eq!(db.pam_not_before("carol").unwrap(), 0);
        db.revoke_pam_sessions("carol", 1000).unwrap();
        db.revoke_pam_sessions("carol", 2000).unwrap();
        assert_eq!(db.pam_not_before("carol").unwrap(), 2000);

        // API tokens.
        let (rec, plain) = db
            .create_api_token(
                "ci",
                Role::User,
                vec!["vm.read".into()],
                Some("2999-01-01T00:00:00Z"),
                Some("alice"),
            )
            .unwrap();
        let f = db.lookup_api_token(&plain).unwrap().unwrap();
        assert_eq!(
            (f.id.as_str(), f.scopes.clone(), f.created_by.as_deref()),
            (rec.id.as_str(), vec!["vm.read".to_string()], Some("alice"))
        );
        assert!(db.lookup_api_token("zrv_not-a-token").unwrap().is_none());
        db.set_api_token_namespaces(&rec.id, Some(&["team-a".to_string()]))
            .unwrap();
        assert_eq!(
            db.lookup_api_token(&plain).unwrap().unwrap().namespaces,
            Some(vec!["team-a".to_string()])
        );
        db.touch_api_token(&rec.id).unwrap();
        assert!(db.list_api_tokens().unwrap()[0].last_used.is_some());
        let (_, expired) = db
            .create_api_token(
                "old",
                Role::Viewer,
                vec![],
                Some("2001-01-01T00:00:00Z"),
                None,
            )
            .unwrap();
        assert!(
            db.lookup_api_token(&expired).unwrap().is_none(),
            "past expiry"
        );
        let (_, junk) = db
            .create_api_token("junk", Role::Viewer, vec![], Some("next tuesday"), None)
            .unwrap();
        assert!(
            db.lookup_api_token(&junk).unwrap().is_none(),
            "unreadable expiry fails closed"
        );
        assert!(db.revoke_api_token(&rec.id).unwrap());
        assert!(!db.revoke_api_token(&rec.id).unwrap());
        assert!(db.lookup_api_token(&plain).unwrap().is_none());
        assert!(db.delete_api_token(&rec.id).unwrap());
        assert_eq!(db.list_api_tokens().unwrap().len(), 2);

        // Deleting a user.
        db.delete_user(&b.id).unwrap();
        assert!(db.get_by_id(&b.id).unwrap().is_none());
    }

    /// What replicas starting or serving at the same time must not get wrong.
    fn replica_races(db: std::sync::Arc<UserDb>) {
        // Both replicas see an empty table and try to seed the admin: exactly one wins.
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let db = db.clone();
                std::thread::spawn(move || {
                    db.seed_admin("admin", "Seed-Password-1").unwrap().is_some()
                })
            })
            .collect();
        let wins = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .filter(|w| *w)
            .count();
        assert_eq!(wins, 1, "exactly one replica seeds the admin");
        assert_eq!(db.count_users().unwrap(), 1);
        // The same OIDC subject provisioned concurrently is one user.
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let db = db.clone();
                std::thread::spawn(move || db.upsert_oidc_user("sub-77", Role::User).unwrap().id)
            })
            .collect();
        let ids: std::collections::HashSet<_> =
            handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(ids.len(), 1, "one OIDC user, not several");
        // One TOTP step is accepted by exactly one of several racing replicas.
        let u = db.get_by_username("admin").unwrap().unwrap();
        db.set_totp(&u.id, "JBSWY3DPEHPK3PXP", true).unwrap();
        let handles: Vec<_> = (0..6)
            .map(|_| {
                let (db, id) = (db.clone(), u.id.clone());
                std::thread::spawn(move || db.consume_totp_step(&id, 555).unwrap())
            })
            .collect();
        let accepted = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .filter(|a| *a)
            .count();
        assert_eq!(accepted, 1, "a TOTP code works once even across replicas");
        // A session revoked through one replica is revoked for the others: both see one counter.
        let tv = db.get_by_id(&u.id).unwrap().unwrap().token_version;
        let handles: Vec<_> = (0..5)
            .map(|_| {
                let (db, id) = (db.clone(), u.id.clone());
                std::thread::spawn(move || db.bump_token_version(&id).unwrap())
            })
            .collect();
        handles.into_iter().for_each(|h| h.join().unwrap());
        assert_eq!(
            db.get_by_id(&u.id).unwrap().unwrap().token_version,
            tv + 5,
            "no lost updates"
        );
    }

    #[test]
    fn sqlite_conformance_and_races() {
        conformance(&UserDb::open(":memory:").unwrap());
        // An in-memory SQLite is private to its connection; the race test uses a file.
        let path = std::env::temp_dir().join(format!("zorvia-races-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        replica_races(std::sync::Arc::new(
            UserDb::open(path.to_str().unwrap()).unwrap(),
        ));
        let _ = std::fs::remove_file(&path);
    }

    /// Needs a throwaway PostgreSQL: `ZORVIA_TEST_POSTGRES_URL=postgres://postgres:pw@127.0.0.1:55432/zorvia`.
    #[test]
    fn postgres_conformance_and_races() {
        let Ok(url) = std::env::var("ZORVIA_TEST_POSTGRES_URL") else {
            eprintln!("skipped: ZORVIA_TEST_POSTGRES_URL is not set");
            return;
        };
        let reset = |url: &str| {
            let db = UserDb::open_postgres(url).unwrap();
            db.db
                .batch("DROP TABLE IF EXISTS users, api_tokens, pam_revocations, login_failures")
                .unwrap();
            UserDb::open_postgres(url).unwrap()
        };
        let db = reset(&url);
        assert!(db.is_postgres());
        conformance(&db);
        replica_races_pg(&url, reset(&url));
        // Importing a SQLite auth database into an empty PostgreSQL store.
        let path = std::env::temp_dir().join(format!("zorvia-import-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let src = UserDb::open(path.to_str().unwrap()).unwrap();
        let u = src
            .create_user("migrated", "password123", Role::Admin)
            .unwrap();
        src.set_namespaces(&u.id, Some(&["team-a".to_string()]))
            .unwrap();
        src.revoke_pam_sessions("pam-user", 42).unwrap();
        let (_, tok) = src
            .create_api_token("ci", Role::User, vec![], None, None)
            .unwrap();
        drop(src);
        let fresh = reset(&url);
        assert_eq!(fresh.import_from_sqlite(path.to_str().unwrap()).unwrap(), 1);
        let m = fresh.get_by_username("migrated").unwrap().unwrap();
        assert_eq!(
            (m.id.as_str(), m.role.clone()),
            (u.id.as_str(), Role::Admin)
        );
        assert!(
            m.verify_password("password123").unwrap(),
            "password hash carried over"
        );
        assert_eq!(
            fresh.get_namespaces(&m.id).unwrap(),
            Some(vec!["team-a".to_string()])
        );
        assert_eq!(fresh.pam_not_before("pam-user").unwrap(), 42);
        assert!(fresh.lookup_api_token(&tok).unwrap().is_some());
        assert!(
            fresh.import_from_sqlite(path.to_str().unwrap()).is_err(),
            "refuses a non-empty target"
        );
        let _ = std::fs::remove_file(&path);
        let _ = fresh
            .db
            .batch("DROP TABLE IF EXISTS users, api_tokens, pam_revocations, login_failures");
    }

    /// Two independent connections (two replicas) on one database.
    fn replica_races_pg(url: &str, first: UserDb) {
        let second = UserDb::open_postgres(url).unwrap();
        let (a, b) = (std::sync::Arc::new(first), std::sync::Arc::new(second));
        // Seed from both at once.
        let (a2, b2) = (a.clone(), b.clone());
        let t1 = std::thread::spawn(move || {
            a2.seed_admin("admin", "Seed-Password-1").unwrap().is_some()
        });
        let t2 = std::thread::spawn(move || {
            b2.seed_admin("admin", "Seed-Password-1").unwrap().is_some()
        });
        let wins = [t1.join().unwrap(), t2.join().unwrap()]
            .iter()
            .filter(|w| **w)
            .count();
        assert_eq!(wins, 1, "two replicas, one admin");
        // A revocation made through one replica is visible through the other immediately.
        let u = a.get_by_username("admin").unwrap().unwrap();
        a.bump_token_version(&u.id).unwrap();
        assert_eq!(b.get_by_id(&u.id).unwrap().unwrap().token_version, 1);
        a.set_enabled(&u.id, false).unwrap();
        assert!(
            !b.get_by_id(&u.id).unwrap().unwrap().enabled,
            "a disabled user is disabled everywhere"
        );
        // The in-process race tests on the shared connection pair.
        a.set_enabled(&u.id, true).unwrap();
        replica_races_shared(a.clone(), b.clone());
    }

    fn replica_races_shared(a: std::sync::Arc<UserDb>, b: std::sync::Arc<UserDb>) {
        let handles: Vec<_> = (0..4)
            .map(|i| {
                let db = if i % 2 == 0 { a.clone() } else { b.clone() };
                std::thread::spawn(move || db.upsert_oidc_user("sub-77", Role::User).unwrap().id)
            })
            .collect();
        let ids: std::collections::HashSet<_> =
            handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(ids.len(), 1, "one OIDC user across replicas");
        let u = a.get_by_username("admin").unwrap().unwrap();
        a.set_totp(&u.id, "JBSWY3DPEHPK3PXP", true).unwrap();
        let handles: Vec<_> = (0..6)
            .map(|i| {
                let db = if i % 2 == 0 { a.clone() } else { b.clone() };
                let id = u.id.clone();
                std::thread::spawn(move || db.consume_totp_step(&id, 555).unwrap())
            })
            .collect();
        assert_eq!(
            handles
                .into_iter()
                .map(|h| h.join().unwrap())
                .filter(|x| *x)
                .count(),
            1,
            "one replica accepts a TOTP step"
        );
        let tv = a.get_by_id(&u.id).unwrap().unwrap().token_version;
        let handles: Vec<_> = (0..6)
            .map(|i| {
                let db = if i % 2 == 0 { a.clone() } else { b.clone() };
                let id = u.id.clone();
                std::thread::spawn(move || db.bump_token_version(&id).unwrap())
            })
            .collect();
        handles.into_iter().for_each(|h| h.join().unwrap());
        assert_eq!(
            b.get_by_id(&u.id).unwrap().unwrap().token_version,
            tv + 6,
            "no lost revocations"
        );
    }
}
