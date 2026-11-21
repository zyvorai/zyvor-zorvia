// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! SAML 2.0 SSO building blocks (Experimental, partial).
//!
//! Opt-in via `ZORVIA_SAML_ENABLED=1`. What is implemented: configuration, SP metadata,
//! SP-initiated AuthnRequest with the HTTP-Redirect binding, the pending-request store,
//! the assertion replay cache, the strict claim validation, and group-attribute role
//! mapping (shared with OIDC).
//!
//! What is deliberately NOT implemented: XML-DSig verification of the IdP response
//! (exclusive C14N, reference/digest checks, signature-wrapping defence). No vetted
//! pure-Rust crate was acceptable (see docs/SAML.md), and this module does not hand-roll
//! canonicalization or signature crypto. Consequently the ACS endpoint fails closed with
//! 501 and **never** mints a session; [`VerifiedAssertion`] is the type a signature
//! verifier must produce, and nothing in this module can construct trust from raw XML.

use std::collections::HashMap;
use std::sync::Mutex;

use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;

use super::groups::{Decision, GroupPolicy};
use super::handlers::SharedAuth;

/// Upper bound on tolerated clock skew, whatever the operator configures.
pub const MAX_CLOCK_SKEW_SECS: i64 = 300;
const DEFAULT_CLOCK_SKEW_SECS: i64 = 60;
/// How long an AuthnRequest ID stays valid for a response.
pub const PENDING_TTL_SECS: i64 = 600;
const MAX_ENTRIES: usize = 10_000;
const MAX_NAME_ID_LEN: usize = 256;

#[derive(Debug, Clone)]
pub struct SamlConfig {
    pub sp_entity_id: String,
    pub acs_url: String,
    pub idp_entity_id: String,
    pub idp_sso_url: String,
    pub groups_attribute: String,
    pub clock_skew_secs: i64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SamlError {
    Config(String),
    Invalid(&'static str),
    Replay,
}

impl std::fmt::Display for SamlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SamlError::Config(m) => write!(f, "SAML configuration: {m}"),
            SamlError::Invalid(m) => write!(f, "SAML response rejected: {m}"),
            SamlError::Replay => write!(f, "SAML response rejected: assertion replayed"),
        }
    }
}

fn require_url(name: &str, v: &str) -> Result<(), SamlError> {
    let local = v.starts_with("http://localhost") || v.starts_with("http://127.0.0.1");
    if !(v.starts_with("https://") || local) || v.chars().any(|c| c.is_control() || c == ' ') {
        return Err(SamlError::Config(format!(
            "{name} must be an https URL (http only for localhost)"
        )));
    }
    Ok(())
}

impl SamlConfig {
    /// `Ok(None)` unless `ZORVIA_SAML_ENABLED` is `1`/`true`; a broken config when enabled
    /// is an error so startup fails rather than guessing.
    pub fn from_env() -> Result<Option<Self>, SamlError> {
        Self::from_lookup(|k| std::env::var(k).ok())
    }

    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Option<Self>, SamlError> {
        let on = get("ZORVIA_SAML_ENABLED")
            .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true"))
            .unwrap_or(false);
        if !on {
            return Ok(None);
        }
        let req = |k: &str| {
            get(k)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .ok_or_else(|| SamlError::Config(format!("{k} is required")))
        };
        let cfg = Self {
            sp_entity_id: req("ZORVIA_SAML_SP_ENTITY_ID")?,
            acs_url: req("ZORVIA_SAML_ACS_URL")?,
            idp_entity_id: req("ZORVIA_SAML_IDP_ENTITY_ID")?,
            idp_sso_url: req("ZORVIA_SAML_IDP_SSO_URL")?,
            groups_attribute: get("ZORVIA_SAML_GROUPS_ATTRIBUTE")
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| "groups".into()),
            clock_skew_secs: match get("ZORVIA_SAML_CLOCK_SKEW_SECS") {
                None => DEFAULT_CLOCK_SKEW_SECS,
                Some(v) => v
                    .trim()
                    .parse::<i64>()
                    .ok()
                    .filter(|n| (0..=MAX_CLOCK_SKEW_SECS).contains(n))
                    .ok_or_else(|| {
                        SamlError::Config(format!(
                            "ZORVIA_SAML_CLOCK_SKEW_SECS must be 0..={MAX_CLOCK_SKEW_SECS}"
                        ))
                    })?,
            },
        };
        require_url("ZORVIA_SAML_ACS_URL", &cfg.acs_url)?;
        require_url("ZORVIA_SAML_IDP_SSO_URL", &cfg.idp_sso_url)?;
        Ok(Some(cfg))
    }
}

pub fn xml_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            c => o.push(c),
        }
    }
    o
}

/// SP metadata (unsigned): POST ACS, `WantAssertionsSigned`, requests are not signed.
pub fn sp_metadata_xml(cfg: &SamlConfig) -> String {
    format!(
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>",
            "<md:EntityDescriptor xmlns:md=\"urn:oasis:names:tc:SAML:2.0:metadata\" entityID=\"{e}\">",
            "<md:SPSSODescriptor AuthnRequestsSigned=\"false\" WantAssertionsSigned=\"true\" ",
            "protocolSupportEnumeration=\"urn:oasis:names:tc:SAML:2.0:protocol\">",
            "<md:NameIDFormat>urn:oasis:names:tc:SAML:2.0:nameid-format:persistent</md:NameIDFormat>",
            "<md:AssertionConsumerService Binding=\"urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST\" ",
            "Location=\"{a}\" index=\"0\" isDefault=\"true\"/>",
            "</md:SPSSODescriptor></md:EntityDescriptor>"
        ),
        e = xml_escape(&cfg.sp_entity_id),
        a = xml_escape(&cfg.acs_url)
    )
}

/// A fresh request ID: an NCName must not start with a digit, so prefix `_`.
pub fn new_request_id() -> String {
    format!("_{}", super::oidc::random_urlsafe(24).replace('-', "a"))
}

pub fn authn_request_xml(cfg: &SamlConfig, id: &str, now: i64) -> String {
    let instant = chrono::DateTime::from_timestamp(now, 0)
        .unwrap_or_default()
        .format("%Y-%m-%dT%H:%M:%SZ");
    format!(
        concat!(
            "<samlp:AuthnRequest xmlns:samlp=\"urn:oasis:names:tc:SAML:2.0:protocol\" ",
            "xmlns:saml=\"urn:oasis:names:tc:SAML:2.0:assertion\" ID=\"{id}\" Version=\"2.0\" ",
            "IssueInstant=\"{t}\" Destination=\"{d}\" ",
            "ProtocolBinding=\"urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST\" ",
            "AssertionConsumerServiceURL=\"{a}\">",
            "<saml:Issuer>{e}</saml:Issuer></samlp:AuthnRequest>"
        ),
        id = xml_escape(id),
        t = instant,
        d = xml_escape(&cfg.idp_sso_url),
        a = xml_escape(&cfg.acs_url),
        e = xml_escape(&cfg.sp_entity_id)
    )
}

/// Raw DEFLATE using one stored (uncompressed) block, which every inflater accepts;
/// avoids a compression dependency for a request of a few hundred bytes.
fn deflate_stored(data: &[u8]) -> Option<Vec<u8>> {
    let len = u16::try_from(data.len()).ok()?;
    let mut o = Vec::with_capacity(data.len() + 5);
    o.push(0x01); // BFINAL=1, BTYPE=00
    o.extend_from_slice(&len.to_le_bytes());
    o.extend_from_slice(&(!len).to_le_bytes());
    o.extend_from_slice(data);
    Some(o)
}

/// HTTP-Redirect binding URL: `SAMLRequest=base64(deflate(xml))&RelayState=...`.
pub fn redirect_url(cfg: &SamlConfig, xml: &str, relay_state: &str) -> Option<String> {
    let body = STANDARD.encode(deflate_stored(xml.as_bytes())?);
    let sep = if cfg.idp_sso_url.contains('?') {
        '&'
    } else {
        '?'
    };
    Some(format!(
        "{}{sep}SAMLRequest={}&RelayState={}",
        cfg.idp_sso_url,
        urlencoding::encode(&body),
        urlencoding::encode(relay_state)
    ))
}

/// Facts from an assertion whose XML signature has ALREADY been verified against a
/// configured IdP certificate by a vetted verifier (none is wired in yet). Never build
/// this from unverified XML.
#[derive(Debug, Clone, Default)]
pub struct VerifiedAssertion {
    pub assertion_id: String,
    pub issuer: String,
    pub destination: String,
    pub in_response_to: Option<String>,
    pub audiences: Vec<String>,
    pub not_before: Option<i64>,
    pub not_on_or_after: Option<i64>,
    pub name_id: String,
    pub attributes: HashMap<String, Vec<String>>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct SamlIdentity {
    pub name_id: String,
    pub decision: Option<Decision>,
}

/// One-time store of outstanding AuthnRequest IDs (InResponseTo binding).
#[derive(Default)]
pub struct PendingRequests(Mutex<HashMap<String, i64>>);

impl PendingRequests {
    pub fn insert(&self, id: &str, now: i64) -> bool {
        let Ok(mut m) = self.0.lock() else {
            return false;
        };
        m.retain(|_, t| now - *t < PENDING_TTL_SECS);
        if m.len() >= MAX_ENTRIES {
            return false;
        }
        m.insert(id.to_string(), now);
        true
    }

    /// Consumes the ID; `true` only if it was outstanding and unexpired.
    pub fn take(&self, id: &str, now: i64) -> bool {
        let Ok(mut m) = self.0.lock() else {
            return false;
        };
        matches!(m.remove(id), Some(t) if now - t < PENDING_TTL_SECS)
    }
}

/// Seen-assertion cache, entries kept until the assertion itself could no longer validate.
#[derive(Default)]
pub struct ReplayCache(Mutex<HashMap<String, i64>>);

impl ReplayCache {
    /// `true` if newly recorded, `false` if already seen (or the cache is full/poisoned: fail closed).
    pub fn record(&self, id: &str, expires: i64, now: i64) -> bool {
        let Ok(mut m) = self.0.lock() else {
            return false;
        };
        m.retain(|_, e| *e > now);
        if m.contains_key(id) || m.len() >= MAX_ENTRIES {
            return false;
        }
        m.insert(id.to_string(), expires);
        true
    }
}

/// Strict checks on a signature-verified assertion. Stateless checks run first; the
/// pending-request and replay entries are consumed only once everything else holds.
pub fn validate_assertion(
    cfg: &SamlConfig,
    policy: Option<&GroupPolicy>,
    a: &VerifiedAssertion,
    now: i64,
    pending: &PendingRequests,
    replay: &ReplayCache,
) -> Result<SamlIdentity, SamlError> {
    let skew = cfg.clock_skew_secs.clamp(0, MAX_CLOCK_SKEW_SECS);
    if a.assertion_id.is_empty() {
        return Err(SamlError::Invalid("missing assertion ID"));
    }
    if a.issuer != cfg.idp_entity_id {
        return Err(SamlError::Invalid("issuer mismatch"));
    }
    if a.destination != cfg.acs_url {
        return Err(SamlError::Invalid("destination mismatch"));
    }
    if !a.audiences.iter().any(|x| x == &cfg.sp_entity_id) {
        return Err(SamlError::Invalid("audience mismatch"));
    }
    let Some(not_after) = a.not_on_or_after else {
        return Err(SamlError::Invalid("missing NotOnOrAfter"));
    };
    if now >= not_after.saturating_add(skew) {
        return Err(SamlError::Invalid("assertion expired"));
    }
    if matches!(a.not_before, Some(nb) if now.saturating_add(skew) < nb) {
        return Err(SamlError::Invalid("assertion not yet valid"));
    }
    if a.name_id.is_empty() || a.name_id.len() > MAX_NAME_ID_LEN || a.name_id.contains('\0') {
        return Err(SamlError::Invalid("invalid NameID"));
    }
    let Some(irt) = a.in_response_to.as_deref() else {
        // Only SP-initiated flow is supported: unsolicited responses are refused.
        return Err(SamlError::Invalid("unsolicited response (no InResponseTo)"));
    };
    if !pending.take(irt, now) {
        return Err(SamlError::Invalid("InResponseTo unknown or expired"));
    }
    if !replay.record(
        &a.assertion_id,
        not_after.saturating_add(skew).saturating_add(1),
        now,
    ) {
        return Err(SamlError::Replay);
    }
    let decision = policy.map(|p| {
        let vals = a
            .attributes
            .get(&cfg.groups_attribute)
            .cloned()
            .unwrap_or_default();
        p.decide(&vals)
    });
    Ok(SamlIdentity {
        name_id: a.name_id.clone(),
        decision,
    })
}

// ── HTTP handlers ─────────────────────────────────────────────────────────

fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

fn off() -> Response {
    (StatusCode::NOT_FOUND, "SAML is not enabled").into_response()
}

pub async fn saml_metadata_handler(State(auth): State<SharedAuth>) -> Response {
    let Some(s) = auth.saml.as_ref() else {
        return off();
    };
    (
        [(header::CONTENT_TYPE, "application/samlmetadata+xml")],
        sp_metadata_xml(&s.cfg),
    )
        .into_response()
}

pub async fn saml_login_handler(State(auth): State<SharedAuth>) -> Response {
    let Some(s) = auth.saml.as_ref() else {
        return off();
    };
    let now = now_unix();
    let id = new_request_id();
    if !s.pending.insert(&id, now) {
        return (StatusCode::SERVICE_UNAVAILABLE, "too many pending sign-ins").into_response();
    }
    let xml = authn_request_xml(&s.cfg, &id, now);
    match redirect_url(&s.cfg, &xml, "") {
        Some(u) => Redirect::to(&u).into_response(),
        None => (StatusCode::INTERNAL_SERVER_ERROR, "cannot build request").into_response(),
    }
}

/// Fails closed: XML-DSig verification is not implemented, so no response is trusted.
pub async fn saml_acs_handler(State(auth): State<SharedAuth>) -> Response {
    if auth.saml.is_none() {
        return off();
    }
    log::warn!("SAML ACS hit but response signature verification is not implemented; refusing");
    (
        StatusCode::NOT_IMPLEMENTED,
        "SAML response verification is not implemented in this build; no session was created",
    )
        .into_response()
}

pub struct SamlState {
    pub cfg: SamlConfig,
    pub pending: PendingRequests,
    pub replay: ReplayCache,
}

impl SamlState {
    pub fn from_env() -> Result<Option<Self>, SamlError> {
        Ok(SamlConfig::from_env()?.map(|cfg| Self {
            cfg,
            pending: PendingRequests::default(),
            replay: ReplayCache::default(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::auth::Role;

    const NOW: i64 = 1_800_000_000;

    fn cfg() -> SamlConfig {
        SamlConfig {
            sp_entity_id: "https://zorvia.example/saml".into(),
            acs_url: "https://zorvia.example/api/v1/auth/saml/acs".into(),
            idp_entity_id: "https://idp.example/entity".into(),
            idp_sso_url: "https://idp.example/sso".into(),
            groups_attribute: "groups".into(),
            clock_skew_secs: 60,
        }
    }

    fn good(pending: &PendingRequests) -> VerifiedAssertion {
        assert!(pending.insert("_req1", NOW - 5));
        let mut attributes = HashMap::new();
        attributes.insert("groups".into(), vec!["zorvia-admins".into()]);
        VerifiedAssertion {
            assertion_id: "_a1".into(),
            issuer: cfg().idp_entity_id,
            destination: cfg().acs_url,
            in_response_to: Some("_req1".into()),
            audiences: vec![cfg().sp_entity_id],
            not_before: Some(NOW - 30),
            not_on_or_after: Some(NOW + 300),
            name_id: "alice@example.com".into(),
            attributes,
        }
    }

    fn run(
        a: &VerifiedAssertion,
        p: &PendingRequests,
        r: &ReplayCache,
    ) -> Result<SamlIdentity, SamlError> {
        validate_assertion(&cfg(), None, a, NOW, p, r)
    }

    #[test]
    fn valid_assertion_passes() {
        let (p, r) = (PendingRequests::default(), ReplayCache::default());
        let a = good(&p);
        assert_eq!(run(&a, &p, &r).unwrap().name_id, "alice@example.com");
    }

    #[test]
    fn replay_is_rejected() {
        let (p, r) = (PendingRequests::default(), ReplayCache::default());
        let a = good(&p);
        run(&a, &p, &r).unwrap();
        // Even with a fresh matching request ID, the same assertion ID is refused.
        assert!(p.insert("_req1", NOW));
        assert_eq!(run(&a, &p, &r), Err(SamlError::Replay));
    }

    #[test]
    fn request_id_is_one_time() {
        let (p, r) = (PendingRequests::default(), ReplayCache::default());
        let a = good(&p);
        run(&a, &p, &r).unwrap();
        let mut b = a.clone();
        b.assertion_id = "_a2".into();
        assert!(
            matches!(run(&b, &p, &r), Err(SamlError::Invalid(m)) if m.contains("InResponseTo"))
        );
    }

    #[test]
    fn wrong_audience_expired_notyet_destination_issuer() {
        let (p, r) = (PendingRequests::default(), ReplayCache::default());
        let ok = good(&p);
        let cases: Vec<(VerifiedAssertion, &str)> = vec![
            (
                VerifiedAssertion {
                    audiences: vec!["https://other".into()],
                    ..ok.clone()
                },
                "audience",
            ),
            (
                VerifiedAssertion {
                    audiences: vec![],
                    ..ok.clone()
                },
                "audience",
            ),
            (
                VerifiedAssertion {
                    not_on_or_after: Some(NOW - 61),
                    ..ok.clone()
                },
                "expired",
            ),
            (
                VerifiedAssertion {
                    not_on_or_after: None,
                    ..ok.clone()
                },
                "NotOnOrAfter",
            ),
            (
                VerifiedAssertion {
                    not_before: Some(NOW + 61),
                    ..ok.clone()
                },
                "not yet",
            ),
            (
                VerifiedAssertion {
                    destination: "https://evil/acs".into(),
                    ..ok.clone()
                },
                "destination",
            ),
            (
                VerifiedAssertion {
                    issuer: "https://evil".into(),
                    ..ok.clone()
                },
                "issuer",
            ),
            (
                VerifiedAssertion {
                    in_response_to: None,
                    ..ok.clone()
                },
                "unsolicited",
            ),
            (
                VerifiedAssertion {
                    name_id: String::new(),
                    ..ok.clone()
                },
                "NameID",
            ),
        ];
        for (a, want) in cases {
            match run(&a, &p, &r) {
                Err(SamlError::Invalid(m)) => assert!(m.contains(want), "{m} vs {want}"),
                other => panic!("expected {want}, got {other:?}"),
            }
        }
        // None of the rejections consumed the pending request.
        run(&ok, &p, &r).unwrap();
    }

    #[test]
    fn skew_boundary_and_clamp() {
        let (p, r) = (PendingRequests::default(), ReplayCache::default());
        let mut a = good(&p);
        a.not_on_or_after = Some(NOW - 59); // within 60s skew
        assert!(run(&a, &p, &r).is_ok());
        // A configured skew above the cap is clamped to 300s.
        let mut c = cfg();
        c.clock_skew_secs = 86_400;
        assert!(p.insert("_req1", NOW));
        let mut b = good(&p);
        b.assertion_id = "_b".into();
        b.not_on_or_after = Some(NOW - 301);
        assert!(validate_assertion(&c, None, &b, NOW, &p, &r).is_err());
    }

    #[test]
    fn unknown_in_response_to_is_rejected() {
        let (p, r) = (PendingRequests::default(), ReplayCache::default());
        let mut a = good(&p);
        a.in_response_to = Some("_forged".into());
        assert!(run(&a, &p, &r).is_err());
    }

    #[test]
    fn groups_use_shared_policy() {
        let (p, r) = (PendingRequests::default(), ReplayCache::default());
        let a = good(&p);
        let pol = GroupPolicy::parse(
            r#"[{"group":"zorvia-admins","role":"admin"}]"#,
            "groups",
            None,
        )
        .unwrap();
        let id = validate_assertion(&cfg(), Some(&pol), &a, NOW, &p, &r).unwrap();
        assert_eq!(
            id.decision,
            Some(Decision::Allow {
                role: Role::Admin,
                namespaces: None
            })
        );
    }

    #[test]
    fn metadata_and_request_are_escaped_and_well_formed() {
        let mut c = cfg();
        c.sp_entity_id = "https://z/\"<x>&".into();
        let m = sp_metadata_xml(&c);
        assert!(m.contains("entityID=\"https://z/&quot;&lt;x&gt;&amp;\""));
        assert!(m.contains("WantAssertionsSigned=\"true\""));
        let x = authn_request_xml(&cfg(), "_id1", NOW);
        assert!(x.contains("ID=\"_id1\"") && x.contains("Destination=\"https://idp.example/sso\""));
        assert!(x.contains("IssueInstant=\"2027-01-15T08:00:00Z\""));
    }

    #[test]
    fn redirect_binding_roundtrips() {
        let c = cfg();
        let xml = authn_request_xml(&c, "_id1", NOW);
        let url = redirect_url(&c, &xml, "a b").unwrap();
        assert!(url.starts_with("https://idp.example/sso?SAMLRequest="));
        assert!(url.ends_with("&RelayState=a%20b"));
        let q = url
            .split("SAMLRequest=")
            .nth(1)
            .unwrap()
            .split('&')
            .next()
            .unwrap();
        let raw = STANDARD
            .decode(urlencoding::decode(q).unwrap().as_bytes())
            .unwrap();
        assert_eq!(raw[0], 0x01);
        let len = u16::from_le_bytes([raw[1], raw[2]]);
        assert_eq!(u16::from_le_bytes([raw[3], raw[4]]), !len);
        assert_eq!(&raw[5..], xml.as_bytes());
    }

    #[test]
    fn config_default_off_and_validated() {
        let none = |_: &str| None;
        assert!(SamlConfig::from_lookup(none).unwrap().is_none());
        let env = |extra: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                let base = [
                    ("ZORVIA_SAML_ENABLED", "1"),
                    ("ZORVIA_SAML_SP_ENTITY_ID", "sp"),
                    ("ZORVIA_SAML_ACS_URL", "https://z/acs"),
                    ("ZORVIA_SAML_IDP_ENTITY_ID", "idp"),
                    ("ZORVIA_SAML_IDP_SSO_URL", "https://idp/sso"),
                ];
                extra
                    .iter()
                    .chain(base.iter())
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| v.to_string())
            }
        };
        assert!(SamlConfig::from_lookup(env(&[])).unwrap().is_some());
        assert!(
            SamlConfig::from_lookup(env(&[("ZORVIA_SAML_ACS_URL", "http://evil/acs")])).is_err()
        );
        assert!(SamlConfig::from_lookup(env(&[("ZORVIA_SAML_CLOCK_SKEW_SECS", "9999")])).is_err());
        assert!(SamlConfig::from_lookup(env(&[("ZORVIA_SAML_IDP_ENTITY_ID", " ")])).is_err());
    }
}
