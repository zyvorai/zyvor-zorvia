// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Small JSON documents (schedules, warm pools, alerts, webhooks, migration history).
//!
//! Each manager loads its document on every request and saves it back, so keeping the
//! document in PostgreSQL (`ZORVIA_DATABASE_URL`) is enough for replicas to see one
//! another's changes. Without a database the document is a file under the data
//! directory, exactly as before.
//!
//! When the database has no row yet, the file is read as a fallback, so switching an
//! existing installation to PostgreSQL carries its documents over the first time each
//! one is saved. Concurrent edits of the *same* document by two replicas are
//! last-writer-wins (see docs/POSTGRES.md).

use anyhow::Result;
use std::path::Path;

#[cfg(feature = "web")]
mod shared {
    use crate::store::Backend;
    use anyhow::Result;
    use std::sync::{Arc, Mutex};

    static DB: Mutex<Option<Arc<Backend>>> = Mutex::new(None);

    /// The shared database, connected lazily (and again after a failure).
    pub fn db() -> Result<Option<Arc<Backend>>> {
        let Some(url) = crate::store::database_url() else {
            return Ok(None);
        };
        let mut guard = DB.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(db) = guard.as_ref() {
            return Ok(Some(db.clone()));
        }
        let db = Backend::postgres(&url)?;
        db.batch(
            "SELECT pg_advisory_xact_lock(727272004);
             CREATE TABLE IF NOT EXISTS state_docs (
                 name TEXT PRIMARY KEY,
                 body TEXT NOT NULL,
                 updated TEXT NOT NULL
             );",
        )?;
        let db = Arc::new(db);
        *guard = Some(db.clone());
        Ok(Some(db))
    }
}

/// The document stored for `path` (keyed by its file name): the database row when a
/// database is configured and has one, otherwise the file.
pub fn read(path: &Path) -> Option<String> {
    #[cfg(feature = "web")]
    {
        match shared::db() {
            Ok(db) => read_with(db.as_deref(), path),
            Err(e) => {
                log::warn!("state document {}: database unavailable: {e}", key(path));
                None
            }
        }
    }
    #[cfg(not(feature = "web"))]
    read_file(path)
}

/// Store the document for `path`.
pub fn write(path: &Path, content: &str) -> Result<()> {
    #[cfg(feature = "web")]
    {
        let db = shared::db()?;
        write_with(db.as_deref(), path, content)
    }
    #[cfg(not(feature = "web"))]
    write_file(path, content)
}

fn read_file(path: &Path) -> Option<String> {
    if path.exists() {
        match std::fs::read_to_string(path) {
            Ok(c) => return Some(c),
            Err(e) => log::warn!("Failed to read {}: {e}", path.display()),
        }
    }
    None
}

fn write_file(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    Ok(())
}

#[cfg(feature = "web")]
fn read_with(db: Option<&crate::store::Backend>, path: &Path) -> Option<String> {
    if let Some(db) = db {
        match db.query_one(
            "SELECT body FROM state_docs WHERE name = ?1",
            &[key(path).into()],
        ) {
            Ok(Some(row)) => return row.opt_text(0),
            Ok(None) => {} // not migrated yet: fall back to the file
            Err(e) => {
                log::warn!("state document {}: database read failed: {e}", key(path));
                return None;
            }
        }
    }
    read_file(path)
}

#[cfg(feature = "web")]
fn write_with(db: Option<&crate::store::Backend>, path: &Path, content: &str) -> Result<()> {
    let Some(db) = db else {
        return write_file(path, content);
    };
    db.exec(
        "INSERT INTO state_docs (name, body, updated) VALUES (?1, ?2, ?3)
         ON CONFLICT (name) DO UPDATE SET body = excluded.body, updated = excluded.updated",
        &[
            key(path).into(),
            content.into(),
            chrono::Utc::now().to_rfc3339().into(),
        ],
    )?;
    Ok(())
}

#[cfg(feature = "web")]
fn key(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}

#[cfg(all(test, feature = "web"))]
mod tests {
    use super::*;

    #[test]
    fn files_are_used_without_a_database() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sub").join("doc.json");
        assert!(read_with(None, &p).is_none());
        write_with(None, &p, "{\"a\":1}").unwrap();
        assert_eq!(read_with(None, &p).unwrap(), "{\"a\":1}");
    }

    /// Needs `ZORVIA_TEST_POSTGRES_URL`: two replicas share documents; an existing file is
    /// the fallback until the first save.
    #[test]
    fn postgres_documents_are_shared_and_fall_back_to_files() {
        let Ok(url) = std::env::var("ZORVIA_TEST_POSTGRES_URL") else {
            eprintln!("skipped: ZORVIA_TEST_POSTGRES_URL is not set");
            return;
        };
        let mk = || {
            let db = crate::store::Backend::postgres(&url).unwrap();
            db.batch(
                "CREATE TABLE IF NOT EXISTS state_docs (name TEXT PRIMARY KEY, body TEXT NOT NULL, updated TEXT NOT NULL)",
            )
            .unwrap();
            db
        };
        let a = mk();
        a.batch("DELETE FROM state_docs").unwrap();
        let b = mk();
        let dir = tempfile::tempdir().unwrap();
        let pa = dir.path().join("a").join("warm_pools.json");
        let pb = dir.path().join("b").join("warm_pools.json");
        // Replica A still has its old file; the database has nothing yet.
        write_file(&pa, "old").unwrap();
        assert_eq!(read_with(Some(&a), &pa).unwrap(), "old");
        assert!(read_with(Some(&b), &pb).is_none());
        // A saves: both replicas now read the database, not their files.
        write_with(Some(&a), &pa, "new").unwrap();
        assert_eq!(read_with(Some(&a), &pa).unwrap(), "new");
        assert_eq!(read_with(Some(&b), &pb).unwrap(), "new");
        write_with(Some(&b), &pb, "newer").unwrap();
        assert_eq!(read_with(Some(&a), &pa).unwrap(), "newer");
        a.batch("DELETE FROM state_docs").unwrap();
    }
}
