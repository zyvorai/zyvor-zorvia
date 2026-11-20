// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Secure OIDC (Authorization Code + PKCE + token exchange + JWKS).
//!
//! Opt-in via `ZORVIA_OIDC_ENABLED=1` plus issuer/client/secret/redirect.
//! Never mints a local JWT without a verified ID token from the IdP.

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// OIDC provider config loaded from env when explicitly enabled.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OidcConfig {
    pub id: String,
    pub name: String,
    pub issuer: String,
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    pub scopes: String,
}

#[derive(Debug, Clone)]
pub struct OidcEndpoints {
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub jwks_uri: String,
    pub issuer: String,
}

#[derive(Debug, Clone)]
pub struct PendingOidc {
    pub code_verifier: String,
    pub nonce: String,
    pub created_unix: u64,
    pub token_endpoint: String,
    pub jwks_uri: String,
    pub issuer: String,
}

#[derive(Debug, Serialize)]
pub struct ProviderInfo {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
struct DiscoveryDocument {
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
    issuer: String,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    id_token: Option<String>,
    #[allow(dead_code)]
    access_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct Jwks {
    keys: Vec<Jwk>,
}

#[derive(Debug, Deserialize, Clone)]
struct Jwk {
    kid: Option<String>,
    kty: String,
    #[serde(rename = "use")]
    #[allow(dead_code)]
    use_: Option<String>,
    n: Option<String>,
    e: Option<String>,
    alg: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct IdTokenClaims {
    pub sub: String,
    #[allow(dead_code)]
    pub iss: String,
    pub aud: Aud,
    pub exp: i64,
    pub iat: Option<i64>,
    pub nbf: Option<i64>,
    /// Authorized party: the client the token was issued to.
    pub azp: Option<String>,
    pub nonce: Option<String>,
    pub email: Option<String>,
    pub preferred_username: Option<String>,
    pub name: Option<String>,
    /// Everything else in the token, for the configurable groups claim.
    #[serde(flatten)]
    pub extra: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Aud {
    One(String),
    Many(Vec<String>),
}

impl Aud {
    fn contains(&self, client_id: &str) -> bool {
        match self {
            Aud::One(a) => a == client_id,
            Aud::Many(v) => v.iter().any(|a| a == client_id),
        }
    }

    fn len(&self) -> usize {
        match self {
            Aud::One(_) => 1,
            Aud::Many(v) => v.len(),
        }
    }
}

/// Clock skew tolerated for `nbf` / `iat`.
const CLOCK_SKEW_SECS: i64 = 120;

/// Claim checks beyond the signature (pure, so it is unit-tested): audience,
/// authorized party (OIDC Core 3.1.3.7: required when there are several
/// audiences, and must be us whenever present), nonce, and the time claims.
pub(crate) fn check_claims(
    claims: &IdTokenClaims,
    client_id: &str,
    expected_nonce: &str,
    now: i64,
) -> anyhow::Result<()> {
    if !claims.aud.contains(client_id) {
        anyhow::bail!("id_token aud does not include client_id");
    }
    match claims.azp.as_deref() {
        Some(azp) if azp != client_id => anyhow::bail!("id_token azp is not this client"),
        None if claims.aud.len() > 1 => {
            anyhow::bail!("id_token has several audiences but no azp")
        }
        _ => {}
    }
    let nonce = claims
        .nonce
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("id_token missing nonce"))?;
    if nonce != expected_nonce {
        anyhow::bail!("id_token nonce mismatch");
    }
    if claims.exp < now {
        anyhow::bail!("id_token expired");
    }
    if claims.nbf.is_some_and(|nbf| nbf > now + CLOCK_SKEW_SECS) {
        anyhow::bail!("id_token is not valid yet (nbf)");
    }
    if claims.iat.is_some_and(|iat| iat > now + CLOCK_SKEW_SECS) {
        anyhow::bail!("id_token was issued in the future (iat)");
    }
    Ok(())
}

impl OidcConfig {
    /// Load OIDC when `ZORVIA_OIDC_ENABLED=1` and required vars are present.
    pub fn from_env() -> Option<Self> {
        let enabled = match std::env::var("ZORVIA_OIDC_ENABLED") {
            Ok(v) => {
                let v = v.trim();
                v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes")
            }
            Err(_) => false,
        };

        let issuer = std::env::var("ZORVIA_OIDC_ISSUER").ok();
        let client_id = std::env::var("ZORVIA_OIDC_CLIENT_ID").ok();

        if !enabled {
            if issuer.is_some() || client_id.is_some() {
                log::warn!(
                    "ZORVIA_OIDC_* is set but OIDC is off; set ZORVIA_OIDC_ENABLED=1 to activate \
                     (requires PKCE + token exchange + JWKS verification)"
                );
            }
            return None;
        }

        let issuer = issuer.filter(|s| !s.trim().is_empty())?;
        let client_id = client_id.filter(|s| !s.trim().is_empty())?;
        let client_secret = std::env::var("ZORVIA_OIDC_CLIENT_SECRET")
            .ok()
            .filter(|s| !s.trim().is_empty())?;
        let redirect_uri = std::env::var("ZORVIA_OIDC_REDIRECT_URI")
            .unwrap_or_else(|_| "https://127.0.0.1:30152/api/v1/auth/oidc/callback".to_string());
        let name = std::env::var("ZORVIA_OIDC_NAME").unwrap_or_else(|_| "OIDC".to_string());
        let scopes = std::env::var("ZORVIA_OIDC_SCOPES")
            .unwrap_or_else(|_| "openid profile email".to_string());

        log::info!(
            "OIDC enabled for issuer={} client_id={} redirect={}",
            issuer,
            client_id,
            redirect_uri
        );

        Some(Self {
            id: "default".into(),
            name,
            issuer: issuer.trim_end_matches('/').to_string(),
            client_id,
            client_secret,
            redirect_uri,
            scopes,
        })
    }

    pub fn discovery_url(&self) -> String {
        format!(
            "{}/.well-known/openid-configuration",
            self.issuer.trim_end_matches('/')
        )
    }
}

pub fn random_urlsafe(nbytes: usize) -> String {
    let mut buf = vec![0u8; nbytes];
    getrandom_fill(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

fn getrandom_fill(buf: &mut [u8]) {
    use rand::Rng;
    rand::rng().fill_bytes(buf);
}

pub fn pkce_challenge_s256(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

pub fn build_authorize_url(
    authorization_endpoint: &str,
    cfg: &OidcConfig,
    state: &str,
    nonce: &str,
    code_challenge: &str,
) -> String {
    let mut url = authorization_endpoint.to_string();
    if !url.contains('?') {
        url.push('?');
    } else if !url.ends_with('&') && !url.ends_with('?') {
        url.push('&');
    }
    url.push_str(&format!(
        "response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&nonce={}&code_challenge={}&code_challenge_method=S256",
        urlencoding::encode(&cfg.client_id),
        urlencoding::encode(&cfg.redirect_uri),
        urlencoding::encode(&cfg.scopes),
        urlencoding::encode(state),
        urlencoding::encode(nonce),
        urlencoding::encode(code_challenge),
    ));
    url
}

pub async fn discover(cfg: &OidcConfig) -> anyhow::Result<OidcEndpoints> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;
    let doc: DiscoveryDocument = client
        .get(cfg.discovery_url())
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    if doc.issuer.trim_end_matches('/') != cfg.issuer.trim_end_matches('/') {
        anyhow::bail!(
            "OIDC discovery issuer mismatch: got {} expected {}",
            doc.issuer,
            cfg.issuer
        );
    }
    Ok(OidcEndpoints {
        authorization_endpoint: doc.authorization_endpoint,
        token_endpoint: doc.token_endpoint,
        jwks_uri: doc.jwks_uri,
        issuer: doc.issuer,
    })
}

pub async fn exchange_code(
    cfg: &OidcConfig,
    token_endpoint: &str,
    code: &str,
    code_verifier: &str,
) -> anyhow::Result<String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()?;
    let basic = STANDARD.encode(format!("{}:{}", cfg.client_id, cfg.client_secret));
    let resp = client
        .post(token_endpoint)
        .header("Authorization", format!("Basic {basic}"))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", cfg.redirect_uri.as_str()),
            ("client_id", cfg.client_id.as_str()),
            ("code_verifier", code_verifier),
        ])
        .send()
        .await?
        .error_for_status()?;
    let body: TokenResponse = resp.json().await?;
    if let Some(err) = body.error {
        anyhow::bail!(
            "token endpoint error: {} ({})",
            err,
            body.error_description.unwrap_or_default()
        );
    }
    body.id_token
        .ok_or_else(|| anyhow::anyhow!("token response missing id_token"))
}

pub async fn verify_id_token(
    id_token: &str,
    cfg: &OidcConfig,
    expected_issuer: &str,
    expected_nonce: &str,
    jwks_uri: &str,
) -> anyhow::Result<IdTokenClaims> {
    let header = decode_header(id_token)?;
    let kid = header.kid;
    // Cached for a few minutes; an unknown `kid` forces one refresh (key rotation).
    let (mut jwks, mut from_cache) = get_jwks(jwks_uri, false).await?;
    let jwk = loop {
        match select_jwk(&jwks, kid.as_deref()) {
            Some(j) => break j,
            None if from_cache => {
                jwks = get_jwks(jwks_uri, true).await?.0;
                from_cache = false;
            }
            None => anyhow::bail!("no matching RSA JWK for id_token"),
        }
    };

    let n = jwk.n.as_deref().unwrap();
    let e = jwk.e.as_deref().unwrap();
    let key = DecodingKey::from_rsa_components(n, e)?;

    let mut validation = Validation::new(Algorithm::RS256);
    if let Some(alg) = jwk.alg.as_deref() {
        if alg == "RS384" {
            validation = Validation::new(Algorithm::RS384);
        } else if alg == "RS512" {
            validation = Validation::new(Algorithm::RS512);
        }
    }
    validation.set_issuer(&[expected_issuer.trim_end_matches('/')]);
    validation.validate_aud = false; // checked manually (string or array)
    validation.set_required_spec_claims(&["sub", "iss", "exp", "aud"]);

    let data = decode::<IdTokenClaims>(id_token, &key, &validation)?;
    let claims = data.claims;

    check_claims(
        &claims,
        &cfg.client_id,
        expected_nonce,
        chrono::Utc::now().timestamp(),
    )?;
    Ok(claims)
}

const JWKS_TTL: std::time::Duration = std::time::Duration::from_secs(300);

type JwksCache = std::sync::Mutex<std::collections::HashMap<String, (std::time::Instant, Jwks)>>;

/// The IdP's key set, from a 5-minute cache unless `force`. The bool says whether
/// it came from the cache.
async fn get_jwks(uri: &str, force: bool) -> anyhow::Result<(Jwks, bool)> {
    static CACHE: std::sync::OnceLock<JwksCache> = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if !force {
        if let Some((at, j)) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(uri) {
            if at.elapsed() < JWKS_TTL {
                return Ok((j.clone(), true));
            }
        }
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;
    let jwks: Jwks = client
        .get(uri)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(uri.to_string(), (std::time::Instant::now(), jwks.clone()));
    Ok((jwks, false))
}

/// Pick the signing key. Without a `kid` we may only guess when the IdP
/// publishes a single RSA key; otherwise any key in the set could be tried.
fn select_jwk(jwks: &Jwks, kid: Option<&str>) -> Option<Jwk> {
    let rsa: Vec<&Jwk> = jwks
        .keys
        .iter()
        .filter(|k| k.kty == "RSA" && k.n.is_some() && k.e.is_some())
        .collect();
    match kid {
        Some(kid) => rsa
            .into_iter()
            .find(|k| k.kid.as_deref() == Some(kid))
            .cloned(),
        None if rsa.len() == 1 => rsa.first().map(|k| (*k).clone()),
        None => None,
    }
}

pub fn display_username(claims: &IdTokenClaims) -> String {
    if let Some(u) = claims
        .preferred_username
        .as_deref()
        .filter(|s| !s.is_empty())
    {
        return u.to_string();
    }
    if let Some(e) = claims.email.as_deref().filter(|s| !s.is_empty()) {
        return e.to_string();
    }
    if let Some(n) = claims.name.as_deref().filter(|s| !s.is_empty()) {
        return n.to_string();
    }
    format!("oidc-{}", claims.sub.chars().take(12).collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claims(aud: Aud, azp: Option<&str>) -> IdTokenClaims {
        IdTokenClaims {
            sub: "u".into(),
            iss: "https://idp".into(),
            aud,
            exp: 2_000,
            iat: Some(1_000),
            nbf: Some(1_000),
            azp: azp.map(str::to_string),
            nonce: Some("n1".into()),
            email: None,
            preferred_username: None,
            name: None,
            extra: Default::default(),
        }
    }

    #[test]
    fn claim_checks_cover_audience_azp_nonce_and_time() {
        let now = 1_500;
        let one = || Aud::One("zorvia".into());
        assert!(check_claims(&claims(one(), None), "zorvia", "n1", now).is_ok());
        assert!(check_claims(&claims(one(), Some("zorvia")), "zorvia", "n1", now).is_ok());
        assert!(check_claims(&claims(one(), Some("other")), "zorvia", "n1", now).is_err());
        assert!(check_claims(&claims(Aud::One("x".into()), None), "zorvia", "n1", now).is_err());
        assert!(check_claims(&claims(one(), None), "zorvia", "wrong", now).is_err());
        // Several audiences need azp, and it must be us.
        let many = || Aud::Many(vec!["zorvia".into(), "other".into()]);
        assert!(check_claims(&claims(many(), None), "zorvia", "n1", now).is_err());
        assert!(check_claims(&claims(many(), Some("zorvia")), "zorvia", "n1", now).is_ok());
        // Time: expired, not yet valid, issued in the future (beyond the skew).
        assert!(check_claims(&claims(one(), None), "zorvia", "n1", 2_001).is_err());
        let mut c = claims(one(), None);
        c.nbf = Some(now + CLOCK_SKEW_SECS + 1);
        assert!(check_claims(&c, "zorvia", "n1", now).is_err());
        c.nbf = Some(now + CLOCK_SKEW_SECS - 1);
        assert!(check_claims(&c, "zorvia", "n1", now).is_ok());
        let mut c = claims(one(), None);
        c.iat = Some(now + CLOCK_SKEW_SECS + 1);
        assert!(check_claims(&c, "zorvia", "n1", now).is_err());
    }

    fn jwk(kid: Option<&str>) -> Jwk {
        Jwk {
            kid: kid.map(str::to_string),
            kty: "RSA".into(),
            use_: None,
            n: Some("n".into()),
            e: Some("e".into()),
            alg: None,
        }
    }

    #[test]
    fn jwk_selection_requires_a_kid_unless_there_is_one_key() {
        let one = Jwks {
            keys: vec![jwk(Some("a"))],
        };
        assert!(select_jwk(&one, None).is_some());
        assert!(select_jwk(&one, Some("a")).is_some());
        assert!(select_jwk(&one, Some("b")).is_none());
        let two = Jwks {
            keys: vec![jwk(Some("a")), jwk(Some("b"))],
        };
        assert!(select_jwk(&two, None).is_none());
        assert_eq!(
            select_jwk(&two, Some("b")).unwrap().kid.as_deref(),
            Some("b")
        );
        let mut ec = jwk(Some("e"));
        ec.kty = "EC".into();
        assert!(select_jwk(&Jwks { keys: vec![ec] }, Some("e")).is_none());
    }

    #[test]
    fn pkce_challenge_is_stable() {
        let v = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let c = pkce_challenge_s256(v);
        assert_eq!(c, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn from_env_disabled_without_flag() {
        // Assumes ZORVIA_OIDC_ENABLED is unset in the unit-test process.
        if std::env::var("ZORVIA_OIDC_ENABLED").is_ok() {
            return;
        }
        assert!(OidcConfig::from_env().is_none());
    }

    #[test]
    fn aud_contains() {
        assert!(Aud::One("a".into()).contains("a"));
        assert!(Aud::Many(vec!["x".into(), "a".into()]).contains("a"));
        assert!(!Aud::One("b".into()).contains("a"));
    }
}
