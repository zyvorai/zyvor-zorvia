# MCP: let AI assistants manage VMs

Zorvia speaks the [Model Context Protocol](https://modelcontextprotocol.io), so an AI assistant (Claude Desktop,
Claude Code, an IDE plugin, your own agent) can list, create, power-cycle and snapshot VMs.

**Security model in one line:** every tool call is replayed through the normal REST API with *the caller's own
credential*. Authentication, role and scope checks, namespace restrictions, rate limiting, audit trail and webhooks
are the REST layer's, unchanged. MCP adds no permissions of its own: an agent can do exactly what its token can do.

## Turn it on

MCP is **off by default**; the endpoint returns 404 until enabled.

```bash
# Helm
helm upgrade --install zorvia charts/zorvia ... --set mcp.enabled=true

# or any deployment
ZORVIA_MCP_ENABLED=1
```

Withhold individual tools with `ZORVIA_MCP_DISABLE_TOOLS=delete_vm,delete_snapshot,revert_snapshot`
(Helm: `--set 'mcp.disableTools={delete_vm,delete_snapshot,revert_snapshot}'`). Disabled tools are neither listed
nor callable.

## Give the agent its own token

Never hand an agent your admin login. Create a scoped, namespace-confined API token (admin only):

```bash
curl -sk -H "Authorization: Bearer $ADMIN_JWT" -H 'content-type: application/json' \
  https://HOST:30152/api/v1/api-tokens \
  -d '{"name":"claude-readonly","role":"viewer","namespaces":["dev"]}'
```

| Goal | Token role | Effect |
|------|-----------|--------|
| Look around, answer questions | `viewer` | `list_vms`, `get_vm`, `list_snapshots`, `cluster_health` work; everything else returns 403 |
| Operate VMs | `user` | adds power, create VM, snapshots. `delete_vm` and `delete_snapshot` still need `vm.delete`, which `user` does not have |
| Full lifecycle | `admin` | everything; use sparingly and prefer `namespaces` |

A token shown once at creation is the only copy; store it in your secret manager.

## Connect a client

**Remote (HTTP) clients:**

```bash
claude mcp add --transport http zorvia https://HOST:30152/api/mcp --header "x-api-key: $ZORVIA_TOKEN"
```

**Clients that launch a local command (stdio):** `zorvia mcp` bridges stdio to the server and holds no
permissions itself.

```bash
export ZORVIA_URL=https://HOST:30152
export ZORVIA_TOKEN=...            # prefer the environment over --token, which shows in `ps`
zorvia mcp --ca-cert ca.pem        # or --insecure for a lab cluster's self-signed certificate
```

Claude Desktop (`claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "zorvia": {
      "command": "zorvia",
      "args": ["mcp", "--ca-cert", "/path/to/ca.pem"],
      "env": { "ZORVIA_URL": "https://HOST:30152", "ZORVIA_TOKEN": "..." }
    }
  }
}
```

## Tools

| Tool | REST call | Needs |
|------|-----------|-------|
| `list_vms` | `GET /vms` | any authenticated |
| `get_vm` | `GET /vms/{name}` | any authenticated |
| `list_snapshots` | `GET /vms/{name}/snapshots` | any authenticated |
| `cluster_health` | `GET /health` | any authenticated |
| `start_vm` / `stop_vm` / `restart_vm` | `POST /vms/{name}/start\|stop\|restart` | `vm.power` |
| `create_vm` | `POST /vms` | `vm.create` |
| `create_snapshot` | `POST /vms/{name}/snapshots` | `vm.create` |
| `revert_snapshot` | `POST /vms/{name}/snapshots/{id}/revert` | `vm.create` |
| `delete_vm` | `DELETE /vms/{name}` | `vm.delete` |
| `delete_snapshot` | `DELETE /vms/{name}/snapshots/{id}` | `vm.delete` |

Safeguards beyond RBAC:

- **Destructive tools need `confirm: true`** (`delete_vm`, `delete_snapshot`, `revert_snapshot`). Without it the
  call is refused and nothing reaches the cluster, so the model has to state the impact and get the user's
  agreement first. Tools carry `readOnlyHint` / `destructiveHint` annotations so clients can prompt too.
- **Names are validated** as plain Kubernetes names; an argument can never redirect a call to another route.
- **`create_vm` forwards only the documented fields.** Advanced options (CPU pinning, hugepages, passthrough)
  stay on the REST API / console.
- Results over 200 000 characters are truncated.

## Audit

Tool calls appear in the audit trail like any REST call, under the token's identity. Filter by the token name to
review everything an agent did.

## Protocol notes

Implements MCP `2025-06-18` (also negotiates `2025-03-26` and `2024-11-05`): `initialize`, `ping`, `tools/list`,
`tools/call` over Streamable HTTP with plain JSON responses (no server-sent events; `GET /api/mcp` returns 405).
Resources, prompts and sampling are not offered.
