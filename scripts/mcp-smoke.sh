#!/usr/bin/env bash
# mcp-smoke.sh: end-to-end check of Zorvia's MCP endpoint against a running server.
#
# It proves the security model, not just that the endpoint answers: a read-only token cannot mutate,
# destructive tools refuse without confirm, argument injection is rejected, credentials are required,
# and (optionally) a real create/snapshot/delete cycle works and shows up in the audit trail.
#
# Usage:
#   ZORVIA_URL=https://HOST:30152 ZORVIA_ADMIN_USER=admin ZORVIA_ADMIN_PASSWORD=... \
#     ./scripts/mcp-smoke.sh
#   ZORVIA_URL=... ZORVIA_ADMIN_TOKEN=<admin jwt or api token> ./scripts/mcp-smoke.sh
#
# Environment:
#   ZORVIA_URL              base URL (required)
#   ZORVIA_ADMIN_TOKEN      admin credential, or:
#   ZORVIA_ADMIN_USER/PASSWORD   login to obtain one
#   ZORVIA_INSECURE=1       accept a self-signed certificate (lab clusters)
#   SMOKE_LIFECYCLE=1       also create, snapshot and delete a throwaway VM (needs a cluster that can run it)
#   SMOKE_IMAGE             image for the lifecycle VM (default quay.io/containerdisks/ubuntu:22.04)
#   SMOKE_NAMESPACE         namespace the restricted tokens are confined to (default: not confined)
#   ZORVIA_BIN              path to the zorvia binary, to also test the stdio bridge
#
# The server must run with ZORVIA_MCP_ENABLED=1 (Helm: --set mcp.enabled=true).
# Exit status: 0 all checks passed, 1 a check failed, 2 prerequisites missing.
set -uo pipefail

PASS=0
FAIL=0
SKIP=0
CLEANUP_TOKENS=()
CLEANUP_VMS=()

red()   { printf '\033[31m%s\033[0m' "$*"; }
green() { printf '\033[32m%s\033[0m' "$*"; }
ok()    { PASS=$((PASS + 1)); printf '  %s %s\n' "$(green PASS)" "$1"; }
bad()   { FAIL=$((FAIL + 1)); printf '  %s %s\n' "$(red FAIL)" "$1"; [ -n "${2:-}" ] && printf '       %s\n' "$2"; }
skip()  { SKIP=$((SKIP + 1)); printf '  SKIP %s\n' "$1"; }
section() { printf '\n== %s\n' "$1"; }

need() { command -v "$1" >/dev/null 2>&1 || { echo "missing prerequisite: $1" >&2; exit 2; }; }
need curl
need jq

[ -n "${ZORVIA_URL:-}" ] || { echo "ZORVIA_URL is required" >&2; exit 2; }
BASE="${ZORVIA_URL%/}"
CURL=(curl -sS --max-time 60)
[ "${ZORVIA_INSECURE:-}" = "1" ] && CURL+=(-k)

# --- HTTP helpers -------------------------------------------------------------------------------

# http METHOD PATH [TOKEN] [BODY]  -> sets HTTP_CODE and HTTP_BODY
http() {
    local method="$1" path="$2" token="${3:-}" body="${4:-}"
    local args=(-X "$method" -o /dev/stdout -w '\n%{http_code}')
    [ -n "$token" ] && args+=(-H "x-api-key: $token")
    if [ -n "$body" ]; then args+=(-H 'content-type: application/json' --data "$body"); fi
    local out
    out="$("${CURL[@]}" "${args[@]}" "$BASE$path" 2>&1)" || true
    HTTP_CODE="${out##*$'\n'}"
    HTTP_BODY="${out%$'\n'*}"
}

# rpc TOKEN METHOD PARAMS_JSON  -> sets HTTP_CODE and HTTP_BODY
rpc() {
    local token="$1" method="$2" params="${3:-null}"
    http POST /api/mcp "$token" \
        "$(jq -nc --arg m "$method" --argjson p "$params" '{jsonrpc:"2.0",id:1,method:$m,params:$p}')"
}

# tool TOKEN NAME ARGS_JSON -> sets HTTP_CODE, HTTP_BODY, TOOL_ERR (true/false) and TOOL_TEXT
tool() {
    local args_json="${3:-}"
    [ -n "$args_json" ] || args_json='{}'
    rpc "$1" tools/call "$(jq -nc --arg n "$2" --argjson a "$args_json" '{name:$n,arguments:$a}')"
    # jq's `//` treats false as empty, so test for null explicitly: isError=false is the success case.
    TOOL_ERR="$(printf '%s' "$HTTP_BODY" | jq -r 'if .result.isError == null then "none" else (.result.isError | tostring) end' 2>/dev/null)"
    TOOL_TEXT="$(printf '%s' "$HTTP_BODY" | jq -r '.result.content[0].text // .error.message // ""' 2>/dev/null)"
}

expect_tool_error() { # label, substring
    if [ "$TOOL_ERR" = "true" ] && printf '%s' "$TOOL_TEXT" | grep -qi -- "$2"; then
        ok "$1"
    else
        bad "$1" "isError=$TOOL_ERR text=${TOOL_TEXT:0:200} (wanted error containing '$2')"
    fi
}

expect_tool_ok() { # label
    if [ "$TOOL_ERR" = "false" ]; then ok "$1"; else bad "$1" "isError=$TOOL_ERR text=${TOOL_TEXT:0:200}"; fi
}

cleanup() {
    section "Cleanup"
    local vm tid
    for vm in "${CLEANUP_VMS[@]:-}"; do
        [ -n "$vm" ] || continue
        http DELETE "/api/vms/$vm" "${ADMIN:-}" >/dev/null
        echo "  deleted VM $vm (HTTP $HTTP_CODE)"
    done
    for tid in "${CLEANUP_TOKENS[@]:-}"; do
        [ -n "$tid" ] || continue
        http POST "/api/v1/api-tokens/$tid" "${ADMIN:-}" >/dev/null
        echo "  revoked token $tid (HTTP $HTTP_CODE)"
    done
}
trap cleanup EXIT

# --- 0. credentials -----------------------------------------------------------------------------

section "Setup"
ADMIN="${ZORVIA_ADMIN_TOKEN:-}"
if [ -z "$ADMIN" ]; then
    if [ -n "${ZORVIA_ADMIN_USER:-}" ] && [ -n "${ZORVIA_ADMIN_PASSWORD:-}" ]; then
        http POST /api/v1/auth/login "" \
            "$(jq -nc --arg u "$ZORVIA_ADMIN_USER" --arg p "$ZORVIA_ADMIN_PASSWORD" '{username:$u,password:$p}')"
        ADMIN="$(printf '%s' "$HTTP_BODY" | jq -r '.token // .data.token // empty' 2>/dev/null)"
    fi
fi
[ -n "$ADMIN" ] || { echo "no admin credential: set ZORVIA_ADMIN_TOKEN or ZORVIA_ADMIN_USER/PASSWORD" >&2; exit 2; }

http GET /api/v1/health ""
[ "$HTTP_CODE" = "200" ] && ok "server healthy" || { bad "server healthy" "HTTP $HTTP_CODE at $BASE"; exit 1; }

rpc "$ADMIN" initialize '{"protocolVersion":"2025-06-18"}'
if [ "$HTTP_CODE" = "404" ] || ! printf '%s' "$HTTP_BODY" | jq -e '.result.serverInfo' >/dev/null 2>&1; then
    bad "MCP endpoint enabled" "HTTP $HTTP_CODE; set ZORVIA_MCP_ENABLED=1 (Helm: --set mcp.enabled=true) and rerun"
    echo; echo "Aborting: nothing further can be tested."; exit 1
fi

# mint_token NAME ROLE -> prints "id token"
mint_token() {
    local ns='null'
    [ -n "${SMOKE_NAMESPACE:-}" ] && ns="$(jq -nc --arg n "$SMOKE_NAMESPACE" '[$n]')"
    http POST /api/v1/api-tokens "$ADMIN" \
        "$(jq -nc --arg n "$1" --arg r "$2" --argjson ns "$ns" \
            '{name:$n,role:$r} + (if $ns==null then {} else {namespaces:$ns} end)')"
    printf '%s %s' "$(printf '%s' "$HTTP_BODY" | jq -r '.id // empty')" "$(printf '%s' "$HTTP_BODY" | jq -r '.token // empty')"
}

RUN="mcp-smoke-$(date +%s)-$RANDOM"
read -r VIEWER_ID VIEWER <<<"$(mint_token "$RUN-viewer" viewer)"
read -r USER_ID USERTOK <<<"$(mint_token "$RUN-user" user)"
CLEANUP_TOKENS+=("$VIEWER_ID" "$USER_ID")
if [ -n "$VIEWER" ] && [ -n "$USERTOK" ]; then ok "minted viewer and user tokens"; else bad "minted viewer and user tokens" "HTTP $HTTP_CODE $HTTP_BODY"; exit 1; fi

# --- 1. authentication --------------------------------------------------------------------------

section "Authentication"
http POST /api/mcp "" '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
[ "$HTTP_CODE" = "401" ] && ok "no credential -> 401" || bad "no credential -> 401" "got HTTP $HTTP_CODE"
http POST /api/mcp "not-a-real-token" '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
[ "$HTTP_CODE" = "401" ] && ok "bad credential -> 401" || bad "bad credential -> 401" "got HTTP $HTTP_CODE"

# --- 2. protocol --------------------------------------------------------------------------------

section "Protocol"
rpc "$VIEWER" initialize '{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}'
V="$(printf '%s' "$HTTP_BODY" | jq -r '.result.protocolVersion')"
[ "$V" = "2025-06-18" ] && ok "initialize negotiates 2025-06-18" || bad "initialize negotiates 2025-06-18" "$HTTP_BODY"
printf '%s' "$HTTP_BODY" | jq -e '.result.serverInfo.name=="zorvia" and .result.capabilities.tools' >/dev/null \
    && ok "serverInfo and tools capability present" || bad "serverInfo and tools capability present" "$HTTP_BODY"
rpc "$VIEWER" initialize '{"protocolVersion":"1999-01-01"}'
[ "$(printf '%s' "$HTTP_BODY" | jq -r '.result.protocolVersion')" = "2025-06-18" ] \
    && ok "unknown client version falls back to latest" || bad "unknown client version falls back to latest" "$HTTP_BODY"

http POST /api/mcp "$VIEWER" '{"jsonrpc":"2.0","method":"notifications/initialized"}'
{ [ "$HTTP_CODE" = "202" ] && [ -z "$HTTP_BODY" ]; } && ok "notification -> 202, empty body" || bad "notification -> 202, empty body" "HTTP $HTTP_CODE '$HTTP_BODY'"
http GET /api/mcp "$VIEWER"
[ "$HTTP_CODE" = "405" ] && ok "GET /api/mcp -> 405 (no event stream)" || bad "GET /api/mcp -> 405" "HTTP $HTTP_CODE"
http POST /api/mcp "$VIEWER" '{broken'
[ "$(printf '%s' "$HTTP_BODY" | jq -r '.error.code' 2>/dev/null)" = "-32700" ] && ok "invalid JSON -> parse error -32700" || bad "invalid JSON -> parse error" "$HTTP_BODY"
rpc "$VIEWER" resources/list
[ "$(printf '%s' "$HTTP_BODY" | jq -r '.error.code')" = "-32601" ] && ok "unsupported method -> -32601" || bad "unsupported method -> -32601" "$HTTP_BODY"
http POST /api/mcp "$VIEWER" '[{"jsonrpc":"2.0","id":1,"method":"ping"},{"jsonrpc":"2.0","method":"notifications/x"}]'
[ "$(printf '%s' "$HTTP_BODY" | jq 'length' 2>/dev/null)" = "1" ] && ok "batch: one reply for one request" || bad "batch: one reply for one request" "$HTTP_BODY"

rpc "$VIEWER" tools/list
N="$(printf '%s' "$HTTP_BODY" | jq '.result.tools | length')"
[ "${N:-0}" -ge 12 ] && ok "tools/list returns $N tools" || bad "tools/list returns >= 12 tools" "$HTTP_BODY"
printf '%s' "$HTTP_BODY" | jq -e 'all(.result.tools[]; .inputSchema.type=="object" and (.annotations.readOnlyHint|type=="boolean"))' >/dev/null \
    && ok "every tool has an object schema and annotations" || bad "every tool has an object schema and annotations"
printf '%s' "$HTTP_BODY" | jq -e '[.result.tools[]|select(.annotations.destructiveHint)|.name]|sort==["delete_snapshot","delete_vm","revert_snapshot"]' >/dev/null \
    && ok "destructive tools flagged: delete_vm, delete_snapshot, revert_snapshot" || bad "destructive tools flagged correctly"
rpc "$VIEWER" tools/call '{"name":"drop_database","arguments":{}}'
[ "$(printf '%s' "$HTTP_BODY" | jq -r '.error.code')" = "-32602" ] && ok "unknown tool -> -32602" || bad "unknown tool -> -32602" "$HTTP_BODY"

# --- 3. read-only token -------------------------------------------------------------------------

section "Viewer token (read-only)"
tool "$VIEWER" cluster_health '{}';        expect_tool_ok "cluster_health works"
tool "$VIEWER" list_vms '{}';              expect_tool_ok "list_vms works"
tool "$VIEWER" list_snapshots '{"name":"mcp-smoke-nonexistent"}'
if [ "$TOOL_ERR" = "false" ] || printf '%s' "$TOOL_TEXT" | grep -q "404"; then ok "list_snapshots reads (200 or 404, never 403)"; else bad "list_snapshots reads" "$TOOL_TEXT"; fi
tool "$VIEWER" start_vm '{"name":"mcp-smoke-nonexistent"}';    expect_tool_error "start_vm denied (403, vm.power)" "403"
tool "$VIEWER" stop_vm '{"name":"mcp-smoke-nonexistent"}';     expect_tool_error "stop_vm denied (403)" "403"
tool "$VIEWER" restart_vm '{"name":"mcp-smoke-nonexistent"}';  expect_tool_error "restart_vm denied (403)" "403"
tool "$VIEWER" create_vm "$(jq -nc --arg n "$RUN-denied" '{name:$n,image:"quay.io/containerdisks/ubuntu:22.04",cpus:1,memory:512}')"
expect_tool_error "create_vm denied (403, vm.create)" "403"
tool "$VIEWER" create_snapshot '{"name":"mcp-smoke-nonexistent","snapshot_name":"s"}'; expect_tool_error "create_snapshot denied (403)" "403"
tool "$VIEWER" delete_vm '{"name":"mcp-smoke-nonexistent","confirm":true}';           expect_tool_error "delete_vm denied even with confirm (403)" "403"
tool "$VIEWER" delete_snapshot '{"name":"mcp-smoke-nonexistent","id":"x","confirm":true}'; expect_tool_error "delete_snapshot denied even with confirm (403)" "403"
tool "$VIEWER" revert_snapshot '{"name":"mcp-smoke-nonexistent","id":"x","confirm":true}'; expect_tool_error "revert_snapshot denied even with confirm (403)" "403"

# --- 4. confirm gate and argument safety (any token) --------------------------------------------

section "Safeguards"
for t in delete_vm delete_snapshot revert_snapshot; do
    tool "$ADMIN" "$t" '{"name":"mcp-smoke-nonexistent","id":"x"}';              expect_tool_error "$t without confirm is refused" "confirm=true"
    tool "$ADMIN" "$t" '{"name":"mcp-smoke-nonexistent","id":"x","confirm":"yes"}'; expect_tool_error "$t with confirm=\"yes\" (not boolean true) is refused" "confirm=true"
done
for evil in '../v1/users' 'a/b' '..' 'a?x=1' 'a%2fb' 'a b' ''; do
    tool "$ADMIN" get_vm "$(jq -nc --arg n "$evil" '{name:$n}')"
    expect_tool_error "get_vm name '$evil' rejected before any request" "Invalid arguments"
done
tool "$ADMIN" get_vm '{}'; expect_tool_error "get_vm without name is rejected" "Invalid arguments\|missing"
tool "$ADMIN" get_vm '{"name":"mcp-smoke-nonexistent"}'; expect_tool_error "get_vm on a missing VM surfaces 404" "404"

# --- 5. user token (can operate, cannot delete) -------------------------------------------------

section "User token (vm.power, vm.create, no vm.delete)"
tool "$USERTOK" list_vms '{}'; expect_tool_ok "list_vms works"
tool "$USERTOK" start_vm '{"name":"mcp-smoke-nonexistent"}'
if [ "$TOOL_ERR" = "true" ] && ! printf '%s' "$TOOL_TEXT" | grep -q "403"; then ok "start_vm passes authorization (fails later with: ${TOOL_TEXT:0:60})"; else bad "start_vm passes authorization" "isError=$TOOL_ERR $TOOL_TEXT"; fi
tool "$USERTOK" delete_vm '{"name":"mcp-smoke-nonexistent","confirm":true}'; expect_tool_error "delete_vm denied (403, needs vm.delete)" "403"
tool "$USERTOK" delete_snapshot '{"name":"mcp-smoke-nonexistent","id":"x","confirm":true}'; expect_tool_error "delete_snapshot denied (403, needs vm.delete)" "403"

# --- 6. revoked credentials stop working immediately --------------------------------------------

section "Revocation"
http POST "/api/v1/api-tokens/$VIEWER_ID" "$ADMIN"
{ [ "$HTTP_CODE" = "204" ] || [ "$HTTP_CODE" = "200" ]; } && ok "viewer token revoked" || bad "viewer token revoked" "HTTP $HTTP_CODE"
rpc "$VIEWER" tools/list
[ "$HTTP_CODE" = "401" ] && ok "revoked token is rejected at once (401)" || bad "revoked token is rejected at once" "HTTP $HTTP_CODE"

# --- 7. real lifecycle (opt-in) -----------------------------------------------------------------

section "Lifecycle (SMOKE_LIFECYCLE=1)"
if [ "${SMOKE_LIFECYCLE:-}" = "1" ]; then
    VM="$RUN"
    CLEANUP_VMS+=("$VM")
    tool "$ADMIN" create_vm "$(jq -nc --arg n "$VM" --arg i "${SMOKE_IMAGE:-quay.io/containerdisks/ubuntu:22.04}" \
        '{name:$n,image:$i,cpus:1,memory:512,disk:5,start:false}')"
    expect_tool_ok "create_vm creates $VM"
    tool "$ADMIN" get_vm "$(jq -nc --arg n "$VM" '{name:$n}')"; expect_tool_ok "get_vm sees the new VM"
    tool "$ADMIN" list_vms '{}'
    printf '%s' "$TOOL_TEXT" | grep -q "$VM" && ok "list_vms includes the new VM" || bad "list_vms includes the new VM"
    tool "$ADMIN" create_snapshot "$(jq -nc --arg n "$VM" '{name:$n,snapshot_name:"smoke-snap",description:"mcp smoke test"}')"
    if [ "$TOOL_ERR" = "false" ]; then ok "create_snapshot accepted"; else skip "create_snapshot (cluster refused: ${TOOL_TEXT:0:80})"; fi
    tool "$ADMIN" delete_vm "$(jq -nc --arg n "$VM" '{name:$n}')"; expect_tool_error "delete_vm without confirm refused; VM still present" "confirm=true"
    tool "$ADMIN" get_vm "$(jq -nc --arg n "$VM" '{name:$n}')"; expect_tool_ok "VM survived the refused delete"
    tool "$ADMIN" delete_vm "$(jq -nc --arg n "$VM" '{name:$n,confirm:true}')"; expect_tool_ok "delete_vm with confirm=true deletes"
    CLEANUP_VMS=()

    section "Audit trail"
    http GET "/api/audit/logs" "$ADMIN"
    if [ "$HTTP_CODE" = "200" ]; then
        printf '%s' "$HTTP_BODY" | grep -q "$VM" && ok "audit trail records actions on $VM" || bad "audit trail records actions on $VM" "no entry mentions $VM"
    else
        skip "audit trail (GET /api/audit/logs -> HTTP $HTTP_CODE)"
    fi
else
    skip "lifecycle: set SMOKE_LIFECYCLE=1 to create/snapshot/delete a throwaway VM"
fi

# --- 8. stdio bridge (opt-in) -------------------------------------------------------------------

section "stdio bridge (ZORVIA_BIN)"
if [ -n "${ZORVIA_BIN:-}" ] && [ -x "${ZORVIA_BIN}" ]; then
    EXTRA=()
    [ "${ZORVIA_INSECURE:-}" = "1" ] && EXTRA+=(--insecure)
    OUT="$(printf '%s\n' \
        '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}' \
        '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
        '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
        | ZORVIA_TOKEN="$USERTOK" "$ZORVIA_BIN" mcp --url "$BASE" "${EXTRA[@]}" 2>/dev/null)"
    [ "$(printf '%s\n' "$OUT" | grep -c .)" = "2" ] && ok "bridge: 2 replies for 2 requests, none for the notification" || bad "bridge reply count" "$OUT"
    printf '%s\n' "$OUT" | tail -n1 | jq -e '.result.tools|length>=12' >/dev/null && ok "bridge: tools/list through stdio" || bad "bridge: tools/list through stdio"
    BAD="$(printf '%s\n' '{"jsonrpc":"2.0","id":3,"method":"ping"}' | ZORVIA_TOKEN=wrong "$ZORVIA_BIN" mcp --url "$BASE" "${EXTRA[@]}" 2>/dev/null)"
    printf '%s' "$BAD" | grep -q "check ZORVIA_TOKEN" && ok "bridge: wrong token explained" || bad "bridge: wrong token explained" "$BAD"
else
    skip "stdio bridge: set ZORVIA_BIN=/path/to/zorvia to test it"
fi

# --- summary ------------------------------------------------------------------------------------

printf '\n%s passed, %s failed, %s skipped\n' "$PASS" "$FAIL" "$SKIP"
[ "$FAIL" -eq 0 ]
