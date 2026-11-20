// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Hot reload of the API server's TLS certificate.
//!
//! cert-manager, a rotated mounted Secret or an operator replacing the files
//! should not need a restart (and the connection drop that comes with one). A
//! background task fingerprints the certificate and key files every
//! `ZORVIA_TLS_RELOAD_SECS` seconds (default 60, 0 turns it off) and, when the
//! content changed, asks the server to reload them. A reload that fails (half-
//! written files, a mismatched key) is logged **once** and the **previous certificate
//! keeps serving**; it is tried again when the files change again.

use sha2::{Digest, Sha256};
use std::future::Future;
use std::path::Path;
use std::time::Duration;

/// SHA-256 over the certificate and key files' contents, or `None` if either cannot
/// be read (mid-rotation); reading the content (not the mtime) also catches
/// Kubernetes' symlink swap on mounted Secrets.
pub fn fingerprint(cert: &Path, key: &Path) -> Option<[u8; 32]> {
    let c = std::fs::read(cert).ok()?;
    let k = std::fs::read(key).ok()?;
    let mut h = Sha256::new();
    h.update((c.len() as u64).to_le_bytes());
    h.update(&c);
    h.update(&k);
    Some(h.finalize().into())
}

pub fn reload_interval() -> Option<Duration> {
    let secs: u64 = std::env::var("ZORVIA_TLS_RELOAD_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);
    (secs > 0).then(|| Duration::from_secs(secs))
}

/// What the watcher remembers between polls.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct WatchState {
    /// Fingerprint of the files currently being served.
    pub applied: Option<[u8; 32]>,
    /// Fingerprint whose reload failed (so the same bad content is not retried and
    /// logged every poll).
    pub failed: Option<[u8; 32]>,
}

/// One poll. Reloads only when the files changed since the last applied *and* last
/// failed content; a failed reload leaves `applied` untouched (the old certificate
/// keeps serving) and records the content so it is not retried until it changes.
pub async fn check_once<F, Fut>(cert: &Path, key: &Path, state: WatchState, reload: F) -> WatchState
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = anyhow::Result<()>>,
{
    let now = fingerprint(cert, key);
    if now.is_none() || now == state.applied || now == state.failed {
        return state;
    }
    match reload().await {
        Ok(()) => {
            log::info!("TLS certificate reloaded from {}", cert.display());
            WatchState {
                applied: now,
                failed: None,
            }
        }
        Err(e) => {
            log::error!("TLS certificate reload failed (keeping the previous one): {e:#}");
            WatchState {
                failed: now,
                ..state
            }
        }
    }
}

/// Run until the process exits.
pub async fn watch(
    config: axum_server::tls_rustls::RustlsConfig,
    cert: String,
    key: String,
    every: Duration,
) {
    let (cp, kp) = (
        Path::new(&cert).to_path_buf(),
        Path::new(&key).to_path_buf(),
    );
    let mut state = WatchState {
        applied: fingerprint(&cp, &kp),
        failed: None,
    };
    let mut tick = tokio::time::interval(every);
    tick.tick().await; // the first tick fires immediately
    loop {
        tick.tick().await;
        state = check_once(&cp, &kp, state, || {
            let config = config.clone();
            let (c, k) = (cp.clone(), kp.clone());
            async move {
                config.reload_from_pem_file(&c, &k).await?;
                Ok(())
            }
        })
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("zorvia-tls-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[tokio::test]
    async fn reloads_only_on_change_and_logs_a_bad_pair_once() {
        let d = tmp("poll");
        let (c, k) = (d.join("tls.crt"), d.join("tls.key"));
        std::fs::write(&c, "cert-1").unwrap();
        std::fs::write(&k, "key-1").unwrap();
        let calls = AtomicUsize::new(0);
        let ok = || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        };
        let bad = || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Err(anyhow::anyhow!("key does not match"))
        };
        // Startup state is taken from the files; an unchanged poll does nothing.
        let s0 = WatchState {
            applied: fingerprint(&c, &k),
            failed: None,
        };
        assert_eq!(check_once(&c, &k, s0, ok).await, s0);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        // The certificate is rotated: one reload, new fingerprint applied.
        std::fs::write(&c, "cert-2").unwrap();
        let s1 = check_once(&c, &k, s0, ok).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_ne!(s1.applied, s0.applied);
        assert_eq!(
            check_once(&c, &k, s1, ok).await,
            s1,
            "same files: no further reload"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // A mismatched key: the reload fails, the served fingerprint is unchanged ...
        std::fs::write(&k, "key-bad").unwrap();
        let s2 = check_once(&c, &k, s1, bad).await;
        assert_eq!(s2.applied, s1.applied);
        assert!(s2.failed.is_some());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        // ... and the same bad content is not retried (or logged) at every poll ...
        assert_eq!(check_once(&c, &k, s2, bad).await, s2);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        // ... until the files change again, which is tried and applied.
        std::fs::write(&k, "key-2").unwrap();
        let s3 = check_once(&c, &k, s2, ok).await;
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert_ne!(s3.applied, s1.applied);
        assert_eq!(s3.failed, None);
        // Going back to exactly what is being served needs no reload.
        std::fs::write(&k, "key-2").unwrap();
        assert_eq!(check_once(&c, &k, s3, ok).await, s3);
        std::fs::remove_dir_all(&d).ok();
    }

    #[tokio::test]
    async fn unreadable_files_mid_rotation_are_ignored() {
        let d = tmp("missing");
        let (c, k) = (d.join("tls.crt"), d.join("tls.key"));
        std::fs::write(&c, "c").unwrap();
        std::fs::write(&k, "k").unwrap();
        let s = WatchState {
            applied: fingerprint(&c, &k),
            failed: None,
        };
        std::fs::remove_file(&k).unwrap(); // symlink swap in progress
        let calls = AtomicUsize::new(0);
        let r = check_once(&c, &k, s, || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .await;
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(r, s, "the last good state is kept");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn the_fingerprint_separates_cert_and_key_content() {
        let d = tmp("fp");
        let (c, k) = (d.join("a"), d.join("b"));
        std::fs::write(&c, "AB").unwrap();
        std::fs::write(&k, "C").unwrap();
        let one = fingerprint(&c, &k);
        std::fs::write(&c, "A").unwrap();
        std::fs::write(&k, "BC").unwrap();
        assert_ne!(
            one,
            fingerprint(&c, &k),
            "moving a byte between files must change it"
        );
        std::fs::remove_dir_all(&d).ok();
    }
}
