// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! cert-manager `Certificate` state and the renew action.
//!
//! Renewal uses cert-manager's documented manual trigger, the same one `cmctl renew`
//! performs: set the `Issuing` condition to `True` (reason `ManuallyTriggered`) through the
//! Certificate's `status` subresource. cert-manager then re-issues and replaces the Secret.
//! Zorvia does **not** delete the Secret (that interrupts consumers and is unnecessary).
//! This needs `get`/`list` on `certificates.cert-manager.io` and `update` on
//! `certificates/status`. Not verified against a live cert-manager.

use super::CertSource;
use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;
use serde_json::{json, Value};

const MESSAGE_MAX: usize = 300;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CmCertificate {
    pub namespace: String,
    pub name: String,
    pub secret_name: Option<String>,
    /// `True`, `False`, `Unknown`, or `Absent` when the Ready condition is missing.
    pub ready: String,
    pub ready_reason: Option<String>,
    pub ready_message: Option<String>,
    /// An issuance is in progress (the `Issuing` condition is `True`).
    pub issuing: bool,
    pub not_after: Option<String>,
    pub renewal_time: Option<String>,
    pub issuer: Option<String>,
    pub dns_names: Vec<String>,
}

fn s(v: &Value) -> Option<String> {
    v.as_str().map(str::to_string)
}

fn condition<'a>(obj: &'a Value, kind: &str) -> Option<&'a Value> {
    obj["status"]["conditions"]
        .as_array()?
        .iter()
        .find(|c| c["type"] == kind)
}

fn truncate(mut m: String) -> String {
    if m.len() > MESSAGE_MAX {
        let mut cut = MESSAGE_MAX;
        while !m.is_char_boundary(cut) {
            cut -= 1;
        }
        m.truncate(cut);
        m.push_str("...");
    }
    m
}

/// Read the fields of one `Certificate` object. `None` for objects without a name.
pub fn parse_certificate(obj: &Value) -> Option<CmCertificate> {
    let name = s(&obj["metadata"]["name"])?;
    let ready = condition(obj, "Ready");
    let issuer = obj["spec"]["issuerRef"]["name"].as_str().map(|n| {
        let kind = obj["spec"]["issuerRef"]["kind"]
            .as_str()
            .unwrap_or("Issuer");
        format!("{kind}/{n}")
    });
    Some(CmCertificate {
        namespace: s(&obj["metadata"]["namespace"]).unwrap_or_default(),
        name,
        secret_name: s(&obj["spec"]["secretName"]),
        ready: ready
            .and_then(|c| s(&c["status"]))
            .unwrap_or_else(|| "Absent".into()),
        ready_reason: ready.and_then(|c| s(&c["reason"])),
        ready_message: ready.and_then(|c| s(&c["message"])).map(truncate),
        issuing: condition(obj, "Issuing").is_some_and(|c| c["status"] == "True"),
        not_after: s(&obj["status"]["notAfter"]),
        renewal_time: s(&obj["status"]["renewalTime"]),
        issuer,
        dns_names: obj["spec"]["dnsNames"]
            .as_array()
            .map(|a| a.iter().filter_map(s).collect())
            .unwrap_or_default(),
    })
}

/// The confirmation phrase a renew request must carry.
pub fn renew_phrase(namespace: &str, name: &str) -> String {
    format!("renew {namespace}/{name}")
}

#[derive(Debug, PartialEq)]
pub enum RenewError {
    /// cert-manager is not installed (its CRD is absent).
    NotInstalled,
    NotFound,
    /// An issuance is already running; nothing was changed.
    AlreadyIssuing,
    /// The object changed while updating (resourceVersion conflict); retry.
    Conflict(String),
    Failed(String),
}

impl std::fmt::Display for RenewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotInstalled => f.write_str("cert-manager is not installed"),
            Self::NotFound => f.write_str("no such cert-manager Certificate"),
            Self::AlreadyIssuing => f.write_str("an issuance is already in progress"),
            Self::Conflict(m) | Self::Failed(m) => f.write_str(m),
        }
    }
}

/// Set `Issuing=True (ManuallyTriggered)`, keeping every other condition. Refuses when an
/// issuance is already running. Mirrors `cmctl renew`.
pub fn mark_issuing(obj: &mut Value, now: DateTime<Utc>) -> Result<(), RenewError> {
    if condition(obj, "Issuing").is_some_and(|c| c["status"] == "True") {
        return Err(RenewError::AlreadyIssuing);
    }
    let mut cond = json!({
        "type": "Issuing",
        "status": "True",
        "reason": "ManuallyTriggered",
        "message": "Certificate re-issuance manually triggered via Zorvia",
        "lastTransitionTime": now.to_rfc3339_opts(SecondsFormat::Secs, true),
    });
    if let Some(g) = obj["metadata"]["generation"].as_i64() {
        cond["observedGeneration"] = json!(g);
    }
    if !obj["status"].is_object() {
        obj["status"] = json!({});
    }
    let conds = obj["status"]["conditions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut conds: Vec<Value> = conds
        .into_iter()
        .filter(|c| c["type"] != "Issuing")
        .collect();
    conds.push(cond);
    obj["status"]["conditions"] = Value::Array(conds);
    Ok(())
}

/// Trigger a renewal. The caller has already validated names and the confirmation phrase.
pub async fn renew(
    source: &impl CertSource,
    namespace: &str,
    name: &str,
    now: DateTime<Utc>,
) -> Result<CmCertificate, RenewError> {
    if source
        .list_certmanager()
        .await
        .map_err(|e| RenewError::Failed(e.to_string()))?
        .is_none()
    {
        return Err(RenewError::NotInstalled);
    }
    let mut obj = source
        .get_certmanager(namespace, name)
        .await
        .map_err(|e| RenewError::Failed(e.to_string()))?
        .ok_or(RenewError::NotFound)?;
    mark_issuing(&mut obj, now)?;
    source
        .replace_certmanager_status(namespace, name, &obj)
        .await
        .map_err(|e| {
            let m = e.to_string();
            if m.contains("Conflict") || m.contains("409") {
                RenewError::Conflict("the Certificate changed while renewing; retry".into())
            } else {
                RenewError::Failed(m)
            }
        })?;
    parse_certificate(&obj).ok_or(RenewError::NotFound)
}

#[cfg(test)]
mod tests {
    use super::super::fake::FakeSource;
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap()
    }

    fn cert_obj(ns: &str, name: &str, ready: &str, issuing: Option<&str>) -> Value {
        let mut conds = vec![
            json!({"type":"Ready","status":ready,"reason":"Ready","message":"Certificate is up to date"}),
        ];
        if let Some(i) = issuing {
            conds.push(json!({"type":"Issuing","status":i,"reason":"Requested"}));
        }
        json!({
            "apiVersion":"cert-manager.io/v1","kind":"Certificate",
            "metadata":{"namespace":ns,"name":name,"generation":3,"resourceVersion":"77"},
            "spec":{"secretName":format!("{name}-tls"),"dnsNames":["a.test"],
                    "issuerRef":{"name":"letsencrypt","kind":"ClusterIssuer"}},
            "status":{"conditions":conds,"notAfter":"2026-09-01T00:00:00Z","renewalTime":"2026-08-01T00:00:00Z"}
        })
    }

    #[test]
    fn parses_ready_state_and_fields() {
        let c = parse_certificate(&cert_obj("web", "site", "True", None)).unwrap();
        assert_eq!(c.ready, "True");
        assert_eq!(c.secret_name.as_deref(), Some("site-tls"));
        assert_eq!(c.issuer.as_deref(), Some("ClusterIssuer/letsencrypt"));
        assert_eq!(c.not_after.as_deref(), Some("2026-09-01T00:00:00Z"));
        assert!(!c.issuing);
        let c = parse_certificate(&cert_obj("web", "site", "False", Some("True"))).unwrap();
        assert_eq!(c.ready, "False");
        assert!(c.issuing);
        let bare = json!({"metadata":{"name":"x","namespace":"n"},"spec":{}});
        assert_eq!(parse_certificate(&bare).unwrap().ready, "Absent");
        assert!(parse_certificate(&json!({})).is_none());
    }

    #[test]
    fn long_messages_are_truncated() {
        let mut o = cert_obj("n", "x", "False", None);
        o["status"]["conditions"][0]["message"] = json!("é".repeat(1000));
        let m = parse_certificate(&o).unwrap().ready_message.unwrap();
        assert!(m.len() <= MESSAGE_MAX + 3);
    }

    #[test]
    fn mark_issuing_keeps_other_conditions_and_refuses_when_running() {
        let mut o = cert_obj("n", "x", "True", Some("False"));
        mark_issuing(&mut o, now()).unwrap();
        let conds = o["status"]["conditions"].as_array().unwrap();
        assert_eq!(conds.len(), 2);
        assert!(conds.iter().any(|c| c["type"] == "Ready"));
        let i = conds.iter().find(|c| c["type"] == "Issuing").unwrap();
        assert_eq!(i["status"], "True");
        assert_eq!(i["reason"], "ManuallyTriggered");
        assert_eq!(i["observedGeneration"], 3);
        assert_eq!(i["lastTransitionTime"], "2026-06-01T12:00:00Z");
        assert_eq!(mark_issuing(&mut o, now()), Err(RenewError::AlreadyIssuing));
        // Status absent entirely.
        let mut bare = json!({"metadata":{"name":"x"}});
        mark_issuing(&mut bare, now()).unwrap();
        assert_eq!(bare["status"]["conditions"][0]["type"], "Issuing");
    }

    #[tokio::test]
    async fn renew_updates_status_through_the_source() {
        let fake = FakeSource::default();
        *fake.certmanager.lock().unwrap() = Some(vec![cert_obj("web", "site", "True", None)]);
        let c = renew(&fake, "web", "site", now()).await.unwrap();
        assert!(c.issuing);
        let rep = fake.replaced.lock().unwrap();
        assert_eq!(rep.len(), 1);
        assert_eq!((rep[0].0.as_str(), rep[0].1.as_str()), ("web", "site"));
        // The resourceVersion travels with the update (optimistic concurrency).
        assert_eq!(rep[0].2["metadata"]["resourceVersion"], "77");
    }

    #[tokio::test]
    async fn renew_error_cases() {
        let none = FakeSource::default();
        assert_eq!(
            renew(&none, "a", "b", now()).await.unwrap_err(),
            RenewError::NotInstalled
        );

        let fake = FakeSource::default();
        *fake.certmanager.lock().unwrap() =
            Some(vec![cert_obj("web", "busy", "False", Some("True"))]);
        assert_eq!(
            renew(&fake, "web", "nope", now()).await.unwrap_err(),
            RenewError::NotFound
        );
        assert_eq!(
            renew(&fake, "web", "busy", now()).await.unwrap_err(),
            RenewError::AlreadyIssuing
        );
        assert!(fake.replaced.lock().unwrap().is_empty());

        let failing = FakeSource {
            replace_error: Some("Conflict: object modified".into()),
            ..Default::default()
        };
        *failing.certmanager.lock().unwrap() = Some(vec![cert_obj("web", "site", "True", None)]);
        assert!(matches!(
            renew(&failing, "web", "site", now()).await,
            Err(RenewError::Conflict(_))
        ));
    }

    #[test]
    fn phrase_is_namespaced() {
        assert_eq!(renew_phrase("web", "site"), "renew web/site");
    }
}
