# SAML 2.0 SSO (Experimental)

Opt-in via `ZORVIA_SAML_ENABLED=1` (default off; also listed as Experimental). Login works **only in a build with the `saml`
cargo feature**. That feature is off by default and the published container image is built without it, so out of the box the
ACS endpoint refuses every response with `501` and never creates a session. **Not tested against any real IdP.**

## Two kinds of build

| Build | ACS (`POST /api/v1/auth/saml/acs`) |
|-------|-----------------------------------|
| Default (no `saml` feature) | fail-closed `501`, no session |
| `cargo build --features saml` | verifies the XML signature, validates the assertion, then signs the user in |

The `saml` feature pulls in the `samael` crate, which links the system library **libxmlsec1** (plus libxml2 and OpenSSL
development files). Signature verification is not hand-rolled: it is delegated to libxmlsec1. The feature is kept optional so
default, `rescue-agent` and container builds do not need those libraries. To use SAML you must build your own image or binary
with `--features saml`; no such image is published.

## How verification works (`saml` build)

* xmlsec verifies every `Signature` against the configured IdP certificate(s) (several are allowed, for key rollover) and then
  reduces the document to the verified content only; unsigned elements are never read. That is the defence against signature
  wrapping. At least one valid signature (on the Response or on the Assertion) is required.
* Algorithms: RSA and ECDSA with SHA-256, SHA-384 or SHA-512. SHA-1 and MD5 signatures are rejected.
* DTDs and entity declarations are refused; the base64 `SAMLResponse` is capped at 512 KiB; encrypted assertions are rejected.
* The response status must be `Success`, the Response and Assertion issuers must agree, and the assertion then passes the strict
  validation below (issuer, destination, audience, NotBefore/NotOnOrAfter with skew, `InResponseTo`, replay cache).
* On success the user is provisioned and signed in through the same federated tail as OIDC (group-to-role mapping reuses
  `ZORVIA_OIDC_GROUP_ROLES` / default role, see [OIDC.md](OIDC.md)); the token arrives as `/sign-in#oidc_token=...`.

## Implemented

| Piece | Status |
|-------|--------|
| `GET /api/v1/auth/saml/metadata` SP metadata (POST ACS, `WantAssertionsSigned=true`) | done |
| `GET /api/v1/auth/saml/login` SP-initiated AuthnRequest, HTTP-Redirect binding (unsigned request) | done |
| Pending request IDs (one-time, 10 min) for `InResponseTo` | done |
| Strict validation of a verified assertion, group attribute to role mapping | done, unit-tested |
| XML signature verification against IdP certificates | done behind the `saml` feature; unit-tested with locally generated signed fixtures (valid, tampered, wrong certificate, unsigned, SHA-1, signature wrapping, DOCTYPE) |
| `POST /api/v1/auth/saml/acs` session creation | done behind the `saml` feature; `501` without it |
| Signed AuthnRequests, SLO, encrypted assertions, IdP metadata import | not done |
| Any real IdP (Entra ID, Okta, Keycloak, ADFS ...) | **not tested** |

The validator takes a `VerifiedAssertion`, the type only the xmlsec-backed verifier can produce. The signed-XML tests need the
`saml` feature (and libxmlsec1) to compile; the default test run does not exercise them.

## Configuration

| Variable | Required | Default |
|----------|----------|---------|
| `ZORVIA_SAML_ENABLED` | yes (`1` / `true`) | off |
| `ZORVIA_SAML_SP_ENTITY_ID` | yes | |
| `ZORVIA_SAML_ACS_URL` | yes (https; http only for localhost) | |
| `ZORVIA_SAML_IDP_ENTITY_ID` | yes | |
| `ZORVIA_SAML_IDP_SSO_URL` | yes (https) | |
| `ZORVIA_SAML_IDP_CERT_PEM` or `ZORVIA_SAML_IDP_CERT_FILE` | yes in a `saml` build (PEM, one or more certificates; the inline value wins; file max 256 KiB); ignored without the feature | |
| `ZORVIA_SAML_GROUPS_ATTRIBUTE` | no | `groups` |
| `ZORVIA_SAML_CLOCK_SKEW_SECS` | no | `60` (max 300) |

A bad value fails startup. The redirect request is encoded with a stored (uncompressed) DEFLATE block, which any
IdP inflates; it is limited to 64 KiB. Not tested against any real IdP.
