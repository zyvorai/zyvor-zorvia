// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! A deliberately small SQL interface: statements with `?1`-style placeholders,
//! parameters that are NULL, 64-bit integers or text, and rows of the same three.
//! That is all the stores need, and it keeps both backends honest about types:
//! Postgres columns holding integers are `BIGINT`, flags are integers, timestamps
//! are RFC 3339 text, exactly as in SQLite.

use anyhow::{anyhow, bail, Context, Result};
use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Null,
    Int(i64),
    Text(String),
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Text(s.to_string())
    }
}
impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::Text(s)
    }
}
impl From<&String> for Value {
    fn from(s: &String) -> Self {
        Value::Text(s.clone())
    }
}
impl From<i64> for Value {
    fn from(n: i64) -> Self {
        Value::Int(n)
    }
}
impl From<u32> for Value {
    fn from(n: u32) -> Self {
        Value::Int(n as i64)
    }
}
impl From<u64> for Value {
    fn from(n: u64) -> Self {
        Value::Int(n as i64)
    }
}
impl From<usize> for Value {
    fn from(n: usize) -> Self {
        Value::Int(n as i64)
    }
}
impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Int(b as i64)
    }
}
impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(o: Option<T>) -> Self {
        o.map(Into::into).unwrap_or(Value::Null)
    }
}

#[derive(Debug, Clone)]
pub struct Row(pub Vec<Value>);

impl Row {
    pub fn opt_text(&self, i: usize) -> Option<String> {
        match self.0.get(i) {
            Some(Value::Text(s)) => Some(s.clone()),
            Some(Value::Int(n)) => Some(n.to_string()),
            _ => None,
        }
    }
    pub fn text(&self, i: usize) -> Result<String> {
        self.opt_text(i)
            .ok_or_else(|| anyhow!("column {i} is NULL or missing"))
    }
    pub fn opt_int(&self, i: usize) -> Option<i64> {
        match self.0.get(i) {
            Some(Value::Int(n)) => Some(*n),
            Some(Value::Text(s)) => s.parse().ok(),
            _ => None,
        }
    }
    /// An integer column, 0 when NULL.
    pub fn int(&self, i: usize) -> i64 {
        self.opt_int(i).unwrap_or(0)
    }
}

impl std::fmt::Debug for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Backend::Sqlite(_) => "Backend::Sqlite",
            Backend::Postgres(_) => "Backend::Postgres",
        })
    }
}

pub enum Backend {
    Sqlite(Mutex<rusqlite::Connection>),
    Postgres(Pg),
}

impl Backend {
    pub fn sqlite(conn: rusqlite::Connection) -> Self {
        Backend::Sqlite(Mutex::new(conn))
    }

    pub fn postgres(url: &str) -> Result<Self> {
        Ok(Backend::Postgres(Pg::connect(url)?))
    }

    pub fn is_postgres(&self) -> bool {
        matches!(self, Backend::Postgres(_))
    }

    /// Run one statement; returns the number of rows it changed.
    pub fn exec(&self, sql: &str, params: &[Value]) -> Result<u64> {
        match self {
            Backend::Sqlite(c) => {
                let conn = c.lock().map_err(|e| anyhow!("{e}"))?;
                let n = conn.execute(
                    sql,
                    rusqlite::params_from_iter(params.iter().map(to_sqlite)),
                )?;
                Ok(n as u64)
            }
            Backend::Postgres(pg) => pg.exec(sql, params),
        }
    }

    pub fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Row>> {
        match self {
            Backend::Sqlite(c) => {
                let conn = c.lock().map_err(|e| anyhow!("{e}"))?;
                let mut stmt = conn.prepare(sql)?;
                let cols = stmt.column_count();
                let mut rows =
                    stmt.query(rusqlite::params_from_iter(params.iter().map(to_sqlite)))?;
                let mut out = Vec::new();
                while let Some(r) = rows.next()? {
                    let mut vals = Vec::with_capacity(cols);
                    for i in 0..cols {
                        vals.push(match r.get_ref(i)? {
                            rusqlite::types::ValueRef::Null => Value::Null,
                            rusqlite::types::ValueRef::Integer(n) => Value::Int(n),
                            rusqlite::types::ValueRef::Text(t) => {
                                Value::Text(String::from_utf8_lossy(t).into_owned())
                            }
                            rusqlite::types::ValueRef::Real(f) => Value::Text(f.to_string()),
                            rusqlite::types::ValueRef::Blob(_) => Value::Null,
                        });
                    }
                    out.push(Row(vals));
                }
                Ok(out)
            }
            Backend::Postgres(pg) => pg.query(sql, params),
        }
    }

    pub fn query_one(&self, sql: &str, params: &[Value]) -> Result<Option<Row>> {
        Ok(self.query(sql, params)?.into_iter().next())
    }

    /// Run several statements without parameters (schema creation).
    pub fn batch(&self, sql: &str) -> Result<()> {
        match self {
            Backend::Sqlite(c) => {
                let conn = c.lock().map_err(|e| anyhow!("{e}"))?;
                conn.execute_batch(sql)?;
                Ok(())
            }
            Backend::Postgres(pg) => pg.batch(sql),
        }
    }
}

fn to_sqlite(v: &Value) -> rusqlite::types::Value {
    match v {
        Value::Null => rusqlite::types::Value::Null,
        Value::Int(n) => rusqlite::types::Value::Integer(*n),
        Value::Text(s) => rusqlite::types::Value::Text(s.clone()),
    }
}

// ── PostgreSQL ───────────────────────────────────────────────────────────────

/// The Postgres client lives on its own small runtime so it can be used from any
/// thread (async handlers, `spawn_blocking`, plain tests) without nesting runtimes.
fn db_runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("zorvia-db")
            .enable_all()
            .build()
            .expect("database runtime")
    })
}

/// How long one database request may take before it is abandoned
/// (`ZORVIA_DATABASE_TIMEOUT_SECS`, 2 to 25, default 8). Without a bound a database that
/// stops answering without closing the connection (a pod that vanished) would hold every
/// request that needs the store until TCP gives up.
fn request_timeout() -> std::time::Duration {
    let secs = std::env::var("ZORVIA_DATABASE_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(8)
        .clamp(2, 25);
    std::time::Duration::from_secs(secs)
}

const TIMEOUT_MSG: &str = "database request timed out";

/// Run a future on the database runtime and wait for it here, for at most the request timeout.
fn run<F, T>(fut: F) -> Result<T>
where
    F: Future<Output = Result<T>> + Send + 'static,
    T: Send + 'static,
{
    let limit = request_timeout();
    let (tx, rx) = std::sync::mpsc::channel();
    db_runtime().spawn(async move {
        let out = match tokio::time::timeout(limit, fut).await {
            Ok(r) => r,
            Err(_) => Err(anyhow!(TIMEOUT_MSG)),
        };
        let _ = tx.send(out);
    });
    rx.recv()
        .map_err(|_| anyhow!("database task ended unexpectedly"))?
}

pub struct Pg {
    url: String,
    client: Mutex<Arc<tokio_postgres::Client>>,
}

/// `sslmode`: unset/`disable` = plaintext; `require` = encrypted without verifying
/// the server (as libpq does); `verify-ca` = verified chain; `verify-full` = verified
/// chain and host name. `ZORVIA_DATABASE_CA_FILE` adds a CA certificate (PEM).
fn tls_for(url: &str) -> Result<Option<postgres_native_tls::MakeTlsConnector>> {
    let mode = url
        .split_once('?')
        .map(|(_, q)| q)
        .unwrap_or("")
        .split('&')
        .find_map(|kv| kv.strip_prefix("sslmode="))
        .unwrap_or("disable")
        .to_ascii_lowercase();
    if mode == "disable" {
        return Ok(None);
    }
    let mut b = native_tls::TlsConnector::builder();
    if let Ok(path) = std::env::var("ZORVIA_DATABASE_CA_FILE") {
        let pem = std::fs::read(&path).with_context(|| format!("read {path}"))?;
        b.add_root_certificate(native_tls::Certificate::from_pem(&pem)?);
    }
    match mode.as_str() {
        "require" => {
            log::warn!("database sslmode=require encrypts the connection but does not verify the server; use verify-full");
            b.danger_accept_invalid_certs(true)
                .danger_accept_invalid_hostnames(true);
        }
        "verify-ca" => {
            b.danger_accept_invalid_hostnames(true);
        }
        "verify-full" => {}
        other => {
            bail!("unsupported sslmode '{other}' (use disable, require, verify-ca or verify-full)")
        }
    }
    Ok(Some(postgres_native_tls::MakeTlsConnector::new(b.build()?)))
}

async fn connect_client(url: &str) -> Result<tokio_postgres::Client> {
    let tls = tls_for(url)?;
    let mut cfg: tokio_postgres::Config = url.parse().context("invalid ZORVIA_DATABASE_URL")?;
    if cfg.get_connect_timeout().is_none() {
        cfg.connect_timeout(std::time::Duration::from_secs(5));
    }
    // Notice a database that vanished without closing the connection.
    cfg.keepalives(true)
        .keepalives_idle(std::time::Duration::from_secs(15));
    macro_rules! go {
        ($tls:expr) => {{
            let (client, conn) = cfg.connect($tls).await.context("connect to PostgreSQL")?;
            tokio::spawn(async move {
                if let Err(e) = conn.await {
                    log::warn!("PostgreSQL connection ended: {e}");
                }
            });
            client
        }};
    }
    Ok(match tls {
        Some(t) => go!(t),
        None => go!(tokio_postgres::NoTls),
    })
}

/// `?1, ?2` -> `$1, $2`.
pub fn translate_placeholders(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len() + 8);
    let mut chars = sql.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '?' && chars.peek().is_some_and(|d| d.is_ascii_digit()) {
            out.push('$');
        } else {
            out.push(c);
        }
    }
    out
}

type PgParam = Box<dyn tokio_postgres::types::ToSql + Sync + Send>;

/// Bind each value as the type the server inferred for that parameter, so a NULL for a
/// BIGINT column is a NULL BIGINT and not a NULL text.
fn to_pg_params(types: &[tokio_postgres::types::Type], params: &[Value]) -> Result<Vec<PgParam>> {
    use tokio_postgres::types::Type;
    if types.len() != params.len() {
        bail!(
            "statement takes {} parameters, {} given",
            types.len(),
            params.len()
        );
    }
    types
        .iter()
        .zip(params)
        .enumerate()
        .map(|(i, (t, v))| -> Result<PgParam> {
            let int_like = *t == Type::INT8 || *t == Type::INT4 || *t == Type::INT2;
            Ok(match (v, int_like) {
                (Value::Null, true) if *t == Type::INT8 => Box::new(Option::<i64>::None),
                (Value::Null, true) if *t == Type::INT4 => Box::new(Option::<i32>::None),
                (Value::Null, true) => Box::new(Option::<i16>::None),
                (Value::Null, false) => Box::new(Option::<String>::None),
                (Value::Int(n), true) if *t == Type::INT8 => Box::new(*n),
                (Value::Int(n), true) if *t == Type::INT4 => {
                    Box::new(i32::try_from(*n).context("integer out of range")?)
                }
                (Value::Int(n), true) => {
                    Box::new(i16::try_from(*n).context("integer out of range")?)
                }
                (Value::Int(n), false) => Box::new(n.to_string()),
                (Value::Text(s), false) => Box::new(s.clone()),
                (Value::Text(s), true) => bail!(
                    "parameter {} is an integer column but got text {s:?}",
                    i + 1
                ),
            })
        })
        .collect()
}

fn pg_row(r: &tokio_postgres::Row) -> Result<Row> {
    use tokio_postgres::types::Type;
    let mut vals = Vec::with_capacity(r.len());
    for (i, col) in r.columns().iter().enumerate() {
        let t = col.type_();
        vals.push(if *t == Type::INT8 {
            r.try_get::<_, Option<i64>>(i)?
                .map(Value::Int)
                .unwrap_or(Value::Null)
        } else if *t == Type::INT4 {
            r.try_get::<_, Option<i32>>(i)?
                .map(|n| Value::Int(n as i64))
                .unwrap_or(Value::Null)
        } else if *t == Type::INT2 {
            r.try_get::<_, Option<i16>>(i)?
                .map(|n| Value::Int(n as i64))
                .unwrap_or(Value::Null)
        } else if *t == Type::BOOL {
            r.try_get::<_, Option<bool>>(i)?
                .map(|b| Value::Int(b as i64))
                .unwrap_or(Value::Null)
        } else if *t == Type::TEXT || *t == Type::VARCHAR || *t == Type::BPCHAR || *t == Type::NAME
        {
            r.try_get::<_, Option<String>>(i)?
                .map(Value::Text)
                .unwrap_or(Value::Null)
        } else {
            bail!(
                "unsupported PostgreSQL column type {t} in column {}",
                col.name()
            );
        });
    }
    Ok(Row(vals))
}

impl Pg {
    pub fn connect(url: &str) -> Result<Self> {
        let u = url.to_string();
        let client = run(async move { connect_client(&u).await })?;
        Ok(Self {
            url: url.to_string(),
            client: Mutex::new(Arc::new(client)),
        })
    }

    fn current(&self) -> Arc<tokio_postgres::Client> {
        self.client
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Run `f` against the client; if the connection was lost, reconnect once and retry.
    fn with_client<T, F, Fut>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: Fn(Arc<tokio_postgres::Client>) -> Fut,
        Fut: Future<Output = Result<T>> + Send + 'static,
    {
        match run(f(self.current())) {
            Ok(v) => Ok(v),
            Err(e) if is_closed(&e) || is_timeout(&e) => {
                let u = self.url.clone();
                let fresh = Arc::new(run(async move { connect_client(&u).await })?);
                *self.client.lock().unwrap_or_else(|e| e.into_inner()) = fresh.clone();
                log::warn!("PostgreSQL connection re-established");
                run(f(fresh))
            }
            Err(e) => Err(e),
        }
    }

    fn exec(&self, sql: &str, params: &[Value]) -> Result<u64> {
        let sql = translate_placeholders(sql);
        let params = params.to_vec();
        self.with_client(move |c| {
            let (sql, params) = (sql.clone(), params.clone());
            async move {
                let stmt = c.prepare(&sql).await?;
                let bound = to_pg_params(stmt.params(), &params)?;
                let refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = bound
                    .iter()
                    .map(|b| &**b as &(dyn tokio_postgres::types::ToSql + Sync))
                    .collect();
                Ok(c.execute(&stmt, &refs).await?)
            }
        })
    }

    fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Row>> {
        let sql = translate_placeholders(sql);
        let params = params.to_vec();
        self.with_client(move |c| {
            let (sql, params) = (sql.clone(), params.clone());
            async move {
                let stmt = c.prepare(&sql).await?;
                let bound = to_pg_params(stmt.params(), &params)?;
                let refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = bound
                    .iter()
                    .map(|b| &**b as &(dyn tokio_postgres::types::ToSql + Sync))
                    .collect();
                let rows = c.query(&stmt, &refs).await?;
                rows.iter().map(pg_row).collect::<Result<Vec<Row>>>()
            }
        })
    }

    fn batch(&self, sql: &str) -> Result<()> {
        let sql = sql.to_string();
        self.with_client(move |c| {
            let sql = sql.clone();
            async move { Ok(c.batch_execute(&sql).await?) }
        })
    }
}

fn is_timeout(e: &anyhow::Error) -> bool {
    e.to_string() == TIMEOUT_MSG
}

fn is_closed(e: &anyhow::Error) -> bool {
    e.chain().any(|c| {
        c.downcast_ref::<tokio_postgres::Error>()
            .is_some_and(|pe| pe.is_closed())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_become_dollar_params() {
        assert_eq!(
            translate_placeholders("UPDATE t SET a = ?1, b = ?2 WHERE id = ?10 AND x = '?'"),
            "UPDATE t SET a = $1, b = $2 WHERE id = $10 AND x = '?'"
        );
    }

    #[test]
    fn values_convert_and_rows_read_back() {
        assert_eq!(Value::from(Some("a")), Value::Text("a".into()));
        assert_eq!(Value::from(None::<&str>), Value::Null);
        assert_eq!(Value::from(true), Value::Int(1));
        assert_eq!(Value::from(7u32), Value::Int(7));
        let r = Row(vec![Value::Text("x".into()), Value::Int(5), Value::Null]);
        assert_eq!(r.text(0).unwrap(), "x");
        assert_eq!(r.int(1), 5);
        assert_eq!(r.int(2), 0);
        assert_eq!(r.opt_text(2), None);
        assert!(r.text(2).is_err());
    }

    #[test]
    fn sqlite_backend_roundtrips_all_three_value_kinds() {
        let b = Backend::sqlite(rusqlite::Connection::open_in_memory().unwrap());
        b.batch("CREATE TABLE t (id TEXT PRIMARY KEY, n INTEGER, note TEXT)")
            .unwrap();
        assert_eq!(
            b.exec(
                "INSERT INTO t VALUES (?1, ?2, ?3)",
                &["a".into(), 5i64.into(), Value::Null]
            )
            .unwrap(),
            1
        );
        let r = b
            .query_one("SELECT id, n, note FROM t WHERE id = ?1", &["a".into()])
            .unwrap()
            .unwrap();
        assert_eq!(
            (r.text(0).unwrap(), r.int(1), r.opt_text(2)),
            ("a".to_string(), 5, None)
        );
        assert!(b
            .query_one("SELECT id FROM t WHERE id = ?1", &["zz".into()])
            .unwrap()
            .is_none());
        assert!(!b.is_postgres());
    }

    #[test]
    fn nulls_and_integers_bind_as_the_servers_parameter_types() {
        use tokio_postgres::types::Type;
        let ok = to_pg_params(
            &[Type::INT8, Type::TEXT, Type::INT8, Type::INT4],
            &[Value::Null, Value::Null, Value::Int(5), Value::Int(7)],
        );
        assert_eq!(ok.unwrap().len(), 4);
        // Wrong arity, text into an integer column, out-of-range, are errors not panics.
        assert!(to_pg_params(&[Type::INT8], &[]).is_err());
        assert!(to_pg_params(&[Type::INT8], &[Value::Text("x".into())]).is_err());
        assert!(to_pg_params(&[Type::INT4], &[Value::Int(i64::MAX)]).is_err());
        // An integer into a text column is sent as its text.
        assert!(to_pg_params(&[Type::TEXT], &[Value::Int(3)]).is_ok());
    }

    #[test]
    fn tls_mode_is_read_from_the_url() {
        assert!(tls_for("postgres://u@h/db").unwrap().is_none());
        assert!(tls_for("postgres://u@h/db?sslmode=disable")
            .unwrap()
            .is_none());
        assert!(tls_for("postgres://u@h/db?sslmode=verify-full")
            .unwrap()
            .is_some());
        assert!(tls_for("postgres://u@h/db?sslmode=banana").is_err());
    }

    #[test]
    fn an_unreachable_database_is_an_error_not_a_hang() {
        let t = std::time::Instant::now();
        let e = Backend::postgres("postgres://u:p@127.0.0.1:1/db")
            .err()
            .unwrap();
        assert!(t.elapsed() < std::time::Duration::from_secs(8), "{e}");
        assert!(is_timeout(&anyhow!(TIMEOUT_MSG)) && !is_timeout(&anyhow!("other")));
    }
}
