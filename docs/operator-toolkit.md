# Operator toolkit

Commands operators reach for once the fleet is running, plus the topic docs.

[Back to the README](../README.md) · [Docs index](README.md)

Commands operators reach for once the fleet is already running — drift,
change plans, guest insight, golden images, Terraform, placement:

```bash
zorvia change drift desired.yaml
zorvia change drift desired.yaml --actual live.yaml --fail-on high -o json
zorvia change plan desired.yaml --vm payments-01 --fail-on-downtime
zorvia guest guest-insight payments-01 --strict
zorvia guest image-bundle --distro ubuntu --version 22.04 --storage-class fast
zorvia dev terraform-scaffold --output ./terraform/zorvia-vm --url https://HOST:30152
zorvia inventory show
zorvia inventory activity
zorvia maintenance maintenance-plan worker-3
zorvia placement placement-advisor --cpu 2 --memory-gib 4
```

| Topic | Doc |
|-------|-----|
| Drift | [docs/DRIFT_GUARD.md](DRIFT_GUARD.md) |
| Change plans | [docs/CHANGE_PLANNER.md](CHANGE_PLANNER.md) |
| Guest insight | [docs/GUEST_INSIGHT.md](GUEST_INSIGHT.md) |
| Golden images | [docs/GOLDEN_IMAGES.md](GOLDEN_IMAGES.md) |
| OIDC SSO | [docs/OIDC.md](OIDC.md) |
| Feature maturity | [docs/FEATURE_MATURITY.md](FEATURE_MATURITY.md) |
| Enterprise plans | [docs/PHASE5_ENTERPRISE.md](PHASE5_ENTERPRISE.md) |
| Upgrade | [docs/UPGRADE.md](UPGRADE.md) |
| Terraform | [docs/TERRAFORM.md](TERRAFORM.md) |
| Kryton | [docs/KRYTON_INTEGRATION.md](KRYTON_INTEGRATION.md) |
| Atlas storage | [docs/ATLAS_INTEGRATION.md](ATLAS_INTEGRATION.md) |
| Inventory | [docs/VCENTER_FEATURE_MATRIX.md](VCENTER_FEATURE_MATRIX.md) |
| Snapshots | [docs/SNAPSHOTS.md](SNAPSHOTS.md) |
| TUI | [docs/INTERACTIVE_TUI.md](INTERACTIVE_TUI.md) |
| Docs index | [docs/README.md](README.md) |
| Command card | [QUICK_REFERENCE.md](../QUICK_REFERENCE.md) |

Kryton: set `KRYTON_URL` (+ usually `KRYTON_TOKEN`, `KRYTON_PROJECT`) on the API pod. Tokens never reach the browser.

Terraform: no HashiCorp registry provider binary yet — scaffold + `schema.json` + `terraform/modules/zorvia_vm` ship today.
