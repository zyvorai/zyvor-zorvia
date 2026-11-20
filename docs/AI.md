# AI-assisted diagnosis

> **Status: Preview.** Off by default. `POST /api/vms/{name}/ai/diagnose` (`vm.power`).

Gathers VM and VMI status, guest-agent insight, recent Kubernetes events and launcher pod logs, redacts them, and asks a model for a diagnosis with evidence and next steps. The VM page has an **AI** tab with a "Diagnose this VM" button.

## Safety

- **Read-only.** The model has no tools; it returns text. Any fix goes through the normal routes with your own token.
- **Redacted.** Evidence passes through `commercial::redact` (same rules as [diagnostic bundles](DIAGNOSTIC_PRIVACY.md)) and is capped at 24 KiB.
- **Transparent.** The response includes `evidence_sent`, the exact redacted payload; the UI shows it under "Evidence sent to the provider".
- **Same access as logs.** Launcher logs are part of the evidence, so the route needs `vm.power`, like `/vms/{name}/logs`.

## Configuration

| Variable | Meaning |
|---|---|
| `ZORVIA_AI_ENABLED` | `1` to enable. Unset returns `503 AI_NOT_CONFIGURED`. |
| `ZORVIA_AI_PROVIDER` | `anthropic` (default) or `openai` (any OpenAI-compatible endpoint, e.g. a self-hosted model) |
| `ZORVIA_AI_API_KEY` | Provider key. Required for `anthropic`; optional for local `openai` endpoints. |
| `ZORVIA_AI_BASE_URL` | Required for `openai`. Optional override for `anthropic`. |
| `ZORVIA_AI_MODEL` | Required for `openai`. Defaults to `claude-sonnet-5-5` for `anthropic`. |

With a hosted provider, redacted evidence leaves the cluster. Use the `openai` provider with a self-hosted model to keep it on-prem.
