// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! SAML 2.0 SSO (Experimental).
//!
//! Opt-in via `ZORVIA_SAML_ENABLED=1`. Implemented here: configuration, SP metadata,
//! SP-initiated AuthnRequest (HTTP-Redirect), the pending-request store, the assertion replay
//! cache, strict claim validation and group-attribute role mapping (shared with OIDC).
//!
//! Response signature verification is NOT hand-rolled: it is delegated to libxmlsec1 through
//! the `samael` crate and exists only when built with the `saml` cargo feature (off by
//! default; needs system libxmlsec1, see docs/SAML.md). Without the feature the ACS endpoint
//! fails closed with 501 and never mints a session. [`VerifiedAssertion`] is only ever built
//! from XML that xmlsec verified against a configured IdP certificate.

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
    /// DER X.509 certificates the IdP signs with (several allowed, for key rollover).
    pub idp_certs_der: Vec<Vec<u8>>,
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

const MAX_CERT_FILE_BYTES: u64 = 256 * 1024;

/// Parse one or more PEM certificates into DER; every block must be a real X.509 certificate.
pub fn parse_pem_certs(pem: &str) -> Result<Vec<Vec<u8>>, SamlError> {
    let certs = openssl::x509::X509::stack_from_pem(pem.as_bytes())
        .map_err(|_| SamlError::Config("IdP certificate is not valid PEM X.509".into()))?;
    if certs.is_empty() {
        return Err(SamlError::Config("no IdP certificate found in PEM".into()));
    }
    certs
        .iter()
        .map(|c| {
            c.to_der()
                .map_err(|_| SamlError::Config("cannot encode IdP certificate".into()))
        })
        .collect()
}

fn load_idp_certs(get: &impl Fn(&str) -> Option<String>) -> Result<Vec<Vec<u8>>, SamlError> {
    let inline = get("ZORVIA_SAML_IDP_CERT_PEM").filter(|v| !v.trim().is_empty());
    let file = get("ZORVIA_SAML_IDP_CERT_FILE").filter(|v| !v.trim().is_empty());
    let pem = match (inline, file) {
        (Some(p), _) => p,
        (None, Some(f)) => {
            let path = f.trim();
            let too_big = std::fs::metadata(path)
                .map(|m| m.len() > MAX_CERT_FILE_BYTES)
                .unwrap_or(false);
            if too_big {
                return Err(SamlError::Config(
                    "ZORVIA_SAML_IDP_CERT_FILE is too large".into(),
                ));
            }
            std::fs::read_to_string(path).map_err(|e| {
                SamlError::Config(format!("cannot read ZORVIA_SAML_IDP_CERT_FILE: {e}"))
            })?
        }
        (None, None) => {
            // Without the `saml` feature nothing can verify a response anyway.
            return if cfg!(feature = "saml") {
                Err(SamlError::Config(
                    "ZORVIA_SAML_IDP_CERT_PEM or ZORVIA_SAML_IDP_CERT_FILE is required".into(),
                ))
            } else {
                Ok(Vec::new())
            };
        }
    };
    parse_pem_certs(&pem)
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
            idp_certs_der: load_idp_certs(&get)?,
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
/// configured IdP certificate by xmlsec (see `verify`). Never build this from unverified XML.
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

#[derive(Debug, serde::Deserialize)]
pub struct AcsForm {
    #[serde(rename = "SAMLResponse")]
    pub saml_response: String,
}

type AcsInput = Result<axum::extract::Form<AcsForm>, axum::extract::rejection::FormRejection>;

/// Without the `saml` feature there is no signature verifier: fail closed.
#[cfg(not(feature = "saml"))]
pub async fn saml_acs_handler(State(auth): State<SharedAuth>, _form: AcsInput) -> Response {
    if auth.saml.is_none() {
        return off();
    }
    log::warn!("SAML ACS hit but this build has no `saml` feature; refusing");
    (
        StatusCode::NOT_IMPLEMENTED,
        "SAML response verification is not available: this build was compiled without the `saml` \
         cargo feature (needs system libxmlsec1); no session was created",
    )
        .into_response()
}

#[cfg(feature = "saml")]
pub async fn saml_acs_handler(State(auth): State<SharedAuth>, form: AcsInput) -> Response {
    use axum::response::Redirect;
    let Some(s) = auth.saml.as_ref() else {
        return off();
    };
    let Ok(axum::extract::Form(form)) = form else {
        return (StatusCode::BAD_REQUEST, "missing SAMLResponse").into_response();
    };
    let reject = |e: SamlError| {
        log::warn!("{e}");
        (StatusCode::UNAUTHORIZED, "SAML sign-in failed").into_response()
    };
    let verified = match verify::verify_response(&s.cfg, &form.saml_response) {
        Ok(v) => v,
        Err(e) => return reject(e),
    };
    let policy = auth.oidc_groups.as_ref();
    let id = match validate_assertion(&s.cfg, policy, &verified, now_unix(), &s.pending, &s.replay)
    {
        Ok(i) => i,
        Err(e) => return reject(e),
    };
    let token = match super::handlers::finish_federated_login(
        &auth,
        "saml",
        &id.name_id,
        &id.name_id,
        id.decision,
    ) {
        Ok(t) => t,
        Err(resp) => return resp,
    };
    // Fragment, not query: never sent to a server, logged, or put in a Referer header.
    Redirect::to(&format!(
        "/sign-in#oidc_token={}",
        urlencoding::encode(&token)
    ))
    .into_response()
}

#[cfg(feature = "saml")]
pub mod verify {
    //! XML-DSig verification through samael/libxmlsec1.
    //!
    //! xmlsec verifies every `Signature` against the configured IdP certificates and then
    //! *reduces* the document to only the verified content (no unsigned ancestors); this
    //! module parses and consumes only that reduced XML, which is what defeats signature
    //! wrapping: unsigned elements are never read. At least one valid signature (on the
    //! Response or on the Assertion) is mandatory; SHA-1/MD5 signatures and digests are not on
    //! the allow-list and are rejected.

    use super::{SamlConfig, SamlError, VerifiedAssertion};
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    use samael::crypto::{
        AllowedSignatureAlgorithm as Alg, CertificateDer, Crypto, CryptoProvider, ReduceMode,
    };
    use samael::schema::{Assertion, Response};
    use std::collections::HashMap;

    /// Upper bound on the base64 SAMLResponse form field.
    pub const MAX_RESPONSE_B64: usize = 512 * 1024;
    const BEARER: &str = "urn:oasis:names:tc:SAML:2.0:cm:bearer";
    const SUCCESS: &str = "urn:oasis:names:tc:SAML:2.0:status:Success";

    /// SHA-2 family only. SHA-1 and MD5 are deliberately absent, and samael cannot be asked for them.
    pub const ALLOWED_ALGORITHMS: &[Alg] = &[
        Alg::RsaSha256,
        Alg::RsaSha384,
        Alg::RsaSha512,
        Alg::EcdsaSha256,
        Alg::EcdsaSha384,
        Alg::EcdsaSha512,
    ];

    fn bad(m: &'static str) -> SamlError {
        SamlError::Invalid(m)
    }

    /// Local name of the root element of well-formed XML (prefix stripped).
    fn root_local_name(xml: &str) -> Option<&str> {
        let mut rest = xml;
        loop {
            let i = rest.find('<')?;
            rest = &rest[i + 1..];
            if rest.starts_with('?') || rest.starts_with('!') {
                continue;
            }
            let end = rest.find(|c: char| c.is_whitespace() || c == '>' || c == '/')?;
            let name = &rest[..end];
            return Some(name.rsplit(':').next().unwrap_or(name));
        }
    }

    pub fn verify_response(cfg: &SamlConfig, b64: &str) -> Result<VerifiedAssertion, SamlError> {
        if b64.len() > MAX_RESPONSE_B64 {
            return Err(bad("SAMLResponse too large"));
        }
        let compact: String = b64.chars().filter(|c| !c.is_whitespace()).collect();
        let raw = STANDARD
            .decode(compact.as_bytes())
            .map_err(|_| bad("SAMLResponse is not base64"))?;
        let xml = String::from_utf8(raw).map_err(|_| bad("SAMLResponse is not UTF-8"))?;
        verify_xml(cfg, &xml)
    }

    pub fn verify_xml(cfg: &SamlConfig, xml: &str) -> Result<VerifiedAssertion, SamlError> {
        // No DTDs / entities: nothing legitimate needs them, and they enable XXE/bombs.
        let lower = xml.to_ascii_lowercase();
        if lower.contains("<!doctype") || lower.contains("<!entity") {
            return Err(bad("DTD/entity declarations are not allowed"));
        }
        if cfg.idp_certs_der.is_empty() {
            return Err(bad("no IdP certificate configured"));
        }
        let certs: Vec<CertificateDer> = cfg
            .idp_certs_der
            .iter()
            .cloned()
            .map(CertificateDer::from)
            .collect();
        // Zero signatures, a bad signature, a foreign certificate, or a disallowed algorithm all
        // surface here as one opaque error.
        let reduced = Crypto::reduce_xml_to_signed_with_allowed_algorithms(
            xml,
            &certs,
            ReduceMode::ValidateAndMarkNoAncestors,
            Some(ALLOWED_ALGORITHMS),
        )
        .map_err(|_| bad("signature verification failed"))?;

        match root_local_name(&reduced) {
            Some("Response") => {
                let r: Response = reduced
                    .parse()
                    .map_err(|_| bad("malformed signed Response"))?;
                if r.encrypted_assertion.is_some() {
                    return Err(bad("encrypted assertions are not supported"));
                }
                if r.status
                    .as_ref()
                    .and_then(|s| s.status_code.value.as_deref())
                    != Some(SUCCESS)
                {
                    return Err(bad("response status is not Success"));
                }
                let a = r.assertion.ok_or_else(|| bad("no signed assertion"))?;
                if let Some(ri) = r.issuer.as_ref().and_then(|i| i.value.as_deref()) {
                    if Some(ri) != a.issuer.value.as_deref() {
                        return Err(bad("response/assertion issuer differ"));
                    }
                }
                to_verified(cfg, &a, r.destination, r.in_response_to)
            }
            Some("Assertion") => {
                let a: Assertion = reduced
                    .parse()
                    .map_err(|_| bad("malformed signed Assertion"))?;
                to_verified(cfg, &a, None, None)
            }
            _ => Err(bad("unexpected signed element")),
        }
    }

    /// `resp_destination` / `resp_irt` come from a *signed* Response element, or are `None` when
    /// only the assertion was signed; then the (signed) SubjectConfirmationData supplies them.
    fn to_verified(
        cfg: &SamlConfig,
        a: &Assertion,
        resp_destination: Option<String>,
        resp_irt: Option<String>,
    ) -> Result<VerifiedAssertion, SamlError> {
        let subject = a.subject.as_ref().ok_or_else(|| bad("missing Subject"))?;
        let name_id = subject
            .name_id
            .as_ref()
            .map(|n| n.value.trim().to_string())
            .ok_or_else(|| bad("missing NameID"))?;
        let scd = subject
            .subject_confirmations
            .iter()
            .flatten()
            .filter(|c| c.method.as_deref() == Some(BEARER))
            .find_map(|c| c.subject_confirmation_data.as_ref())
            .ok_or_else(|| bad("no bearer SubjectConfirmationData"))?;
        if let (Some(d), Some(r)) = (&resp_destination, &scd.recipient) {
            if d != r {
                return Err(bad("destination mismatch"));
            }
        }
        if let (Some(x), Some(y)) = (&resp_irt, &scd.in_response_to) {
            if x != y {
                return Err(bad("InResponseTo mismatch"));
            }
        }
        let cond = a
            .conditions
            .as_ref()
            .ok_or_else(|| bad("missing Conditions"))?;
        // Every AudienceRestriction must name this SP; the validator then compares against it.
        let restrictions = cond.audience_restrictions.as_deref().unwrap_or_default();
        let audiences = if !restrictions.is_empty()
            && restrictions
                .iter()
                .all(|r| r.audience.contains(&cfg.sp_entity_id))
        {
            vec![cfg.sp_entity_id.clone()]
        } else {
            Vec::new()
        };
        let not_on_or_after = [cond.not_on_or_after, scd.not_on_or_after]
            .into_iter()
            .flatten()
            .map(|t| t.timestamp())
            .min();
        let mut attributes: HashMap<String, Vec<String>> = HashMap::new();
        for st in a.attribute_statements.iter().flatten() {
            for at in &st.attributes {
                if let Some(n) = &at.name {
                    attributes
                        .entry(n.clone())
                        .or_default()
                        .extend(at.values.iter().filter_map(|v| v.value.clone()));
                }
            }
        }
        Ok(VerifiedAssertion {
            assertion_id: a.id.clone(),
            issuer: a.issuer.value.clone().unwrap_or_default(),
            destination: resp_destination
                .or_else(|| scd.recipient.clone())
                .unwrap_or_default(),
            in_response_to: resp_irt.or_else(|| scd.in_response_to.clone()),
            audiences,
            not_before: cond.not_before.map(|t| t.timestamp()),
            not_on_or_after,
            name_id,
            attributes,
        })
    }
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
            idp_certs_der: Vec::new(),
        }
    }

    /// Self-signed RSA-2048 test certificate: (cert PEM, private key).
    pub(super) fn test_cert() -> (String, openssl::pkey::PKey<openssl::pkey::Private>) {
        use openssl::asn1::Asn1Time;
        use openssl::bn::BigNum;
        use openssl::hash::MessageDigest;
        use openssl::pkey::PKey;
        use openssl::rsa::Rsa;
        use openssl::x509::{X509Builder, X509NameBuilder};
        let key = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
        let mut n = X509NameBuilder::new().unwrap();
        n.append_entry_by_text("CN", "test-idp").unwrap();
        let n = n.build();
        let mut b = X509Builder::new().unwrap();
        b.set_version(2).unwrap();
        b.set_serial_number(&BigNum::from_u32(1).unwrap().to_asn1_integer().unwrap())
            .unwrap();
        b.set_subject_name(&n).unwrap();
        b.set_issuer_name(&n).unwrap();
        b.set_pubkey(&key).unwrap();
        b.set_not_before(&Asn1Time::days_from_now(0).unwrap())
            .unwrap();
        b.set_not_after(&Asn1Time::days_from_now(30).unwrap())
            .unwrap();
        b.sign(&key, MessageDigest::sha256()).unwrap();
        let pem = String::from_utf8(b.build().to_pem().unwrap()).unwrap();
        (pem, key)
    }

    #[test]
    fn pem_certs_are_validated() {
        let (pem, _) = test_cert();
        assert_eq!(parse_pem_certs(&pem).unwrap().len(), 1);
        let two = format!("{pem}{pem}");
        assert_eq!(parse_pem_certs(&two).unwrap().len(), 2);
        assert!(parse_pem_certs("").is_err());
        assert!(
            parse_pem_certs("-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n")
                .is_err()
        );
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
                if k == "ZORVIA_SAML_IDP_CERT_PEM" && !extra.iter().any(|(n, _)| *n == k) {
                    return Some(test_cert().0);
                }
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

#[cfg(all(test, feature = "saml"))]
mod verify_tests {
    use super::tests::test_cert;
    use super::*;
    use samael::crypto::{Crypto, CryptoProvider};

    const NOW: i64 = 1_800_000_000;
    const RSA_SHA256: &str = "http://www.w3.org/2001/04/xmldsig-more#rsa-sha256";
    const SHA256: &str = "http://www.w3.org/2001/04/xmlenc#sha256";
    const RSA_SHA1: &str = "http://www.w3.org/2000/09/xmldsig#rsa-sha1";
    const SHA1: &str = "http://www.w3.org/2000/09/xmldsig#sha1";

    fn cfg_with(cert_pem: &str) -> SamlConfig {
        SamlConfig {
            sp_entity_id: "https://zorvia.example/saml".into(),
            acs_url: "https://zorvia.example/api/v1/auth/saml/acs".into(),
            idp_entity_id: "https://idp.example/entity".into(),
            idp_sso_url: "https://idp.example/sso".into(),
            groups_attribute: "groups".into(),
            clock_skew_secs: 60,
            idp_certs_der: parse_pem_certs(cert_pem).unwrap(),
        }
    }

    fn ts(t: i64) -> String {
        chrono::DateTime::from_timestamp(t, 0)
            .unwrap()
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string()
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Sign {
        None,
        Assertion,
        Response,
    }

    struct Fx {
        name: &'static str,
        audience: &'static str,
        not_on_or_after: i64,
        sign: Sign,
        sig_alg: &'static str,
        dig_alg: &'static str,
    }

    impl Fx {
        fn ok(sign: Sign) -> Self {
            Fx {
                name: "alice@example.com",
                audience: "https://zorvia.example/saml",
                not_on_or_after: NOW + 300,
                sign,
                sig_alg: RSA_SHA256,
                dig_alg: SHA256,
            }
        }
    }

    fn sig_template(id: &str, f: &Fx) -> String {
        format!(
            concat!(
                "<ds:Signature xmlns:ds=\"http://www.w3.org/2000/09/xmldsig#\"><ds:SignedInfo>",
                "<ds:CanonicalizationMethod Algorithm=\"http://www.w3.org/2001/10/xml-exc-c14n#\"/>",
                "<ds:SignatureMethod Algorithm=\"{sa}\"/><ds:Reference URI=\"#{id}\"><ds:Transforms>",
                "<ds:Transform Algorithm=\"http://www.w3.org/2000/09/xmldsig#enveloped-signature\"/>",
                "<ds:Transform Algorithm=\"http://www.w3.org/2001/10/xml-exc-c14n#\"/></ds:Transforms>",
                "<ds:DigestMethod Algorithm=\"{da}\"/><ds:DigestValue></ds:DigestValue></ds:Reference>",
                "</ds:SignedInfo><ds:SignatureValue></ds:SignatureValue></ds:Signature>"
            ),
            sa = f.sig_alg,
            da = f.dig_alg,
            id = id
        )
    }

    fn assertion(f: &Fx, id: &str, template: &str) -> String {
        format!(
            concat!(
                "<saml:Assertion xmlns:saml=\"urn:oasis:names:tc:SAML:2.0:assertion\" ID=\"{id}\" ",
                "Version=\"2.0\" IssueInstant=\"{iat}\"><saml:Issuer>https://idp.example/entity</saml:Issuer>{sig}",
                "<saml:Subject><saml:NameID>{name}</saml:NameID>",
                "<saml:SubjectConfirmation Method=\"urn:oasis:names:tc:SAML:2.0:cm:bearer\">",
                "<saml:SubjectConfirmationData NotOnOrAfter=\"{noa}\" ",
                "Recipient=\"https://zorvia.example/api/v1/auth/saml/acs\" InResponseTo=\"_req1\"/>",
                "</saml:SubjectConfirmation></saml:Subject>",
                "<saml:Conditions NotBefore=\"{nb}\" NotOnOrAfter=\"{noa}\"><saml:AudienceRestriction>",
                "<saml:Audience>{aud}</saml:Audience></saml:AudienceRestriction></saml:Conditions>",
                "<saml:AttributeStatement><saml:Attribute Name=\"groups\">",
                "<saml:AttributeValue>zorvia-admins</saml:AttributeValue></saml:Attribute></saml:AttributeStatement>",
                "</saml:Assertion>"
            ),
            id = id,
            iat = ts(NOW - 10),
            sig = template,
            name = f.name,
            noa = ts(f.not_on_or_after),
            nb = ts(NOW - 30),
            aud = f.audience
        )
    }

    fn response_around(inner: &str, template: &str) -> String {
        format!(
            concat!(
                "<samlp:Response xmlns:samlp=\"urn:oasis:names:tc:SAML:2.0:protocol\" ",
                "xmlns:saml=\"urn:oasis:names:tc:SAML:2.0:assertion\" ID=\"_resp1\" Version=\"2.0\" ",
                "IssueInstant=\"{iat}\" Destination=\"https://zorvia.example/api/v1/auth/saml/acs\" ",
                "InResponseTo=\"_req1\"><saml:Issuer>https://idp.example/entity</saml:Issuer>{sig}",
                "<samlp:Status><samlp:StatusCode Value=\"urn:oasis:names:tc:SAML:2.0:status:Success\"/>",
                "</samlp:Status>{inner}</samlp:Response>"
            ),
            iat = ts(NOW - 10),
            sig = template,
            inner = inner
        )
    }

    fn sign(xml: &str, key: &openssl::pkey::PKey<openssl::pkey::Private>) -> String {
        Crypto::sign_xml(xml, &key.private_key_to_der().unwrap()).unwrap()
    }

    fn build(f: &Fx, key: &openssl::pkey::PKey<openssl::pkey::Private>) -> String {
        let xml = match f.sign {
            Sign::None => response_around(&assertion(f, "_a1", ""), ""),
            Sign::Assertion => response_around(&assertion(f, "_a1", &sig_template("_a1", f)), ""),
            Sign::Response => response_around(&assertion(f, "_a1", ""), &sig_template("_resp1", f)),
        };
        if f.sign == Sign::None {
            xml
        } else {
            sign(&xml, key)
        }
    }

    fn run(cfg: &SamlConfig, xml: &str) -> Result<SamlIdentity, SamlError> {
        let v = verify::verify_xml(cfg, xml)?;
        let (p, r) = (PendingRequests::default(), ReplayCache::default());
        assert!(p.insert("_req1", NOW - 5));
        validate_assertion(cfg, None, &v, NOW, &p, &r)
    }

    #[test]
    fn valid_assertion_and_response_signatures_are_accepted() {
        let (pem, key) = test_cert();
        let cfg = cfg_with(&pem);
        for sign in [Sign::Assertion, Sign::Response] {
            let id = run(&cfg, &build(&Fx::ok(sign), &key)).unwrap();
            assert_eq!(id.name_id, "alice@example.com");
        }
        // Groups attribute survives verification.
        let v = verify::verify_xml(&cfg, &build(&Fx::ok(Sign::Assertion), &key)).unwrap();
        assert_eq!(v.attributes["groups"], vec!["zorvia-admins".to_string()]);
    }

    #[test]
    fn base64_form_field_path_works_and_size_is_capped() {
        use base64::Engine;
        let (pem, key) = test_cert();
        let cfg = cfg_with(&pem);
        let xml = build(&Fx::ok(Sign::Assertion), &key);
        let b64 = base64::engine::general_purpose::STANDARD.encode(xml);
        assert!(verify::verify_response(&cfg, &b64).is_ok());
        let wrapped = b64
            .as_bytes()
            .chunks(76)
            .map(|c| std::str::from_utf8(c).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(verify::verify_response(&cfg, &wrapped).is_ok());
        assert!(verify::verify_response(&cfg, &"A".repeat(verify::MAX_RESPONSE_B64 + 1)).is_err());
        assert!(verify::verify_response(&cfg, "not base64!").is_err());
    }

    #[test]
    fn tampered_content_is_rejected() {
        let (pem, key) = test_cert();
        let cfg = cfg_with(&pem);
        for sign in [Sign::Assertion, Sign::Response] {
            let xml = build(&Fx::ok(sign), &key);
            let t = xml.replace("alice@example.com", "mallory@example.com");
            assert_ne!(t, xml);
            assert!(matches!(
                verify::verify_xml(&cfg, &t),
                Err(SamlError::Invalid(m)) if m.contains("signature")
            ));
            // Escalating the group after signing also breaks the digest.
            assert!(
                verify::verify_xml(&cfg, &xml.replace("zorvia-admins", "zorvia-root")).is_err()
            );
        }
    }

    #[test]
    fn wrong_certificate_is_rejected() {
        let (_, key) = test_cert();
        let (other_pem, _) = test_cert();
        let cfg = cfg_with(&other_pem);
        let xml = build(&Fx::ok(Sign::Assertion), &key);
        assert!(verify::verify_xml(&cfg, &xml).is_err());
    }

    #[test]
    fn unsigned_response_is_rejected() {
        let (pem, key) = test_cert();
        let cfg = cfg_with(&pem);
        assert!(matches!(
            verify::verify_xml(&cfg, &build(&Fx::ok(Sign::None), &key)),
            Err(SamlError::Invalid(m)) if m.contains("signature")
        ));
        // A signature element with no value / a stripped signature is no better.
        let signed = build(&Fx::ok(Sign::Assertion), &key);
        let start = signed.find("<ds:Signature").unwrap();
        let end = signed.find("</ds:Signature>").unwrap() + "</ds:Signature>".len();
        let stripped = format!("{}{}", &signed[..start], &signed[end..]);
        assert!(verify::verify_xml(&cfg, &stripped).is_err());
        // No certificate configured: fail closed.
        let mut none = cfg.clone();
        none.idp_certs_der.clear();
        assert!(verify::verify_xml(&none, &signed).is_err());
    }

    #[test]
    fn sha1_signatures_are_rejected_by_default() {
        let (pem, key) = test_cert();
        let cfg = cfg_with(&pem);
        let mut f = Fx::ok(Sign::Assertion);
        f.sig_alg = RSA_SHA1;
        f.dig_alg = SHA1;
        // Sanity: the fixture really is signed with SHA-1.
        let xml = build(&f, &key);
        assert!(xml.contains("rsa-sha1"));
        assert!(verify::verify_xml(&cfg, &xml).is_err());
        // SHA-256 signature with a SHA-1 digest is rejected too.
        let mut g = Fx::ok(Sign::Assertion);
        g.dig_alg = SHA1;
        assert!(verify::verify_xml(&cfg, &build(&g, &key)).is_err());
    }

    #[test]
    fn signature_wrapping_never_yields_the_unsigned_identity() {
        let (pem, key) = test_cert();
        let cfg = cfg_with(&pem);
        // A genuine assertion signed for alice.
        let alice = Fx::ok(Sign::Assertion);
        let signed_assertion = sign(
            &assertion(&alice, "_a1", &sig_template("_a1", &alice)),
            &key,
        );
        let signed_assertion = signed_assertion
            .split_once("?>")
            .map(|(_, r)| r.trim().to_string())
            .unwrap_or(signed_assertion);
        // Attacker prepends an unsigned assertion for mallory where a consumer would look first,
        // and keeps the signed one elsewhere in the (unsigned) Response.
        let mallory = Fx {
            name: "mallory@example.com",
            ..Fx::ok(Sign::None)
        };
        let evil = assertion(&mallory, "_evil", "");
        for xml in [
            response_around(&format!("{evil}{signed_assertion}"), ""),
            response_around(&format!("{signed_assertion}{evil}"), ""),
            response_around(
                &format!("<samlp:Extensions xmlns:samlp=\"urn:oasis:names:tc:SAML:2.0:protocol\">{signed_assertion}</samlp:Extensions>{evil}"),
                "",
            ),
        ] {
            match verify::verify_xml(&cfg, &xml) {
                Ok(v) => assert_eq!(v.name_id, "alice@example.com", "consumed unsigned content"),
                Err(_) => {}
            }
        }
        // Reusing a response signature over a different ID (signature points at "_a1" but the
        // consumed assertion has another ID) must not verify.
        let ok = build(&Fx::ok(Sign::Assertion), &key);
        let swapped = ok.replace("ID=\"_a1\"", "ID=\"_other\"");
        assert!(verify::verify_xml(&cfg, &swapped).is_err());
    }

    #[test]
    fn doctype_and_entities_are_refused() {
        let (pem, key) = test_cert();
        let cfg = cfg_with(&pem);
        let xml = build(&Fx::ok(Sign::Assertion), &key);
        let with_dtd = format!("<!DOCTYPE x [<!ENTITY e \"v\">]>{xml}");
        assert!(matches!(
            verify::verify_xml(&cfg, &with_dtd),
            Err(SamlError::Invalid(m)) if m.contains("DTD")
        ));
    }

    #[test]
    fn replay_expired_and_wrong_audience_after_verification() {
        let (pem, key) = test_cert();
        let cfg = cfg_with(&pem);
        let (p, r) = (PendingRequests::default(), ReplayCache::default());
        let xml = build(&Fx::ok(Sign::Response), &key);
        assert!(p.insert("_req1", NOW - 5));
        let v = verify::verify_xml(&cfg, &xml).unwrap();
        assert!(validate_assertion(&cfg, None, &v, NOW, &p, &r).is_ok());
        // The same signed response again, even with a fresh request ID, is a replay.
        assert!(p.insert("_req1", NOW));
        let v = verify::verify_xml(&cfg, &xml).unwrap();
        assert_eq!(
            validate_assertion(&cfg, None, &v, NOW, &p, &r),
            Err(SamlError::Replay)
        );

        let mut expired = Fx::ok(Sign::Assertion);
        expired.not_on_or_after = NOW - 3600;
        assert!(matches!(
            run(&cfg, &build(&expired, &key)),
            Err(SamlError::Invalid(m)) if m.contains("expired")
        ));

        let mut aud = Fx::ok(Sign::Assertion);
        aud.audience = "https://someone-else";
        assert!(matches!(
            run(&cfg, &build(&aud, &key)),
            Err(SamlError::Invalid(m)) if m.contains("audience")
        ));
    }
}
