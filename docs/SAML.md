# SAML 2.0 SSO (Experimental, partial)

Opt-in via `ZORVIA_SAML_ENABLED=1` (default off; also listed as Experimental). **Signing in with SAML does not work yet:**
the ACS endpoint refuses every response with `501` and never creates a session.

## Why not complete

A SAML login is only as safe as its XML signature check (exclusive C14N, reference and digest validation,
signature-wrapping defence). Candidates were assessed: `samael` needs the C library libxmlsec1 (not installable
here), and the pure-Rust options (`saml-rs` 0.8, `risaml`, `saml` 0.1 alpha, `xml-sec`) are months old, rely on a
newly written XML-DSig stack and pull ~100 additional crates including pre-release crypto crates (`x448`
0.14-pre, `rsa` release candidates) into `Cargo.lock`. `cargo deny` is not available on the build host, so licences
and advisories could not be checked by the real tool. Rather than hand-roll canonicalization or signature crypto
or adopt an unreviewed tree, this release implements only the parts that are safe without it. Choosing and
vetting a verifier (and running `cargo deny check`) is the remaining work.

## Implemented

| Piece | Status |
|-------|--------|
| `GET /api/v1/auth/saml/metadata` SP metadata (POST ACS, `WantAssertionsSigned=true`) | done |
| `GET /api/v1/auth/saml/login` SP-initiated AuthnRequest, HTTP-Redirect binding (unsigned request) | done |
| Pending request IDs (one-time, 10 min) for `InResponseTo` | done |
| Strict validation of a *verified* assertion: issuer, destination, audience, NotBefore/NotOnOrAfter with skew (cap 300 s), `InResponseTo`, replay cache | done, unit-tested |
| Group attribute to role mapping (reuses `ZORVIA_OIDC_GROUP_ROLES` / default role, see [OIDC.md](OIDC.md)) | done, unit-tested |
| `POST /api/v1/auth/saml/acs` | fail-closed `501` |
| XML signature verification against IdP certificates, XML parsing of the Response | **not done** |
| Session creation (the OIDC JIT-user/JWT path) from a SAML identity | **not done** |
| Signed AuthnRequests, SLO, encrypted assertions, IdP metadata import | not done |

The validator takes a `VerifiedAssertion`, the type a future signature verifier must produce; it cannot be built
from raw XML in this module. Tests use fixture structs (valid, wrong audience, expired, not-yet-valid, wrong
destination/issuer, unsolicited, unknown `InResponseTo`, replayed); there are no signed-XML fixtures, so
tampered-signature and wrapping tests do not exist yet.

## Configuration

| Variable | Required | Default |
|----------|----------|---------|
| `ZORVIA_SAML_ENABLED` | yes (`1` / `true`) | off |
| `ZORVIA_SAML_SP_ENTITY_ID` | yes | |
| `ZORVIA_SAML_ACS_URL` | yes (https; http only for localhost) | |
| `ZORVIA_SAML_IDP_ENTITY_ID` | yes | |
| `ZORVIA_SAML_IDP_SSO_URL` | yes (https) | |
| `ZORVIA_SAML_GROUPS_ATTRIBUTE` | no | `groups` |
| `ZORVIA_SAML_CLOCK_SKEW_SECS` | no | `60` (max 300) |

A bad value fails startup. The redirect request is encoded with a stored (uncompressed) DEFLATE block, which any
IdP inflates; it is limited to 64 KiB. Not tested against any real IdP.
