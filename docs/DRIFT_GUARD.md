# Zorvia Drift Guard

Drift Guard performs semantic desired-vs-actual comparison for KubeVirt `VirtualMachine` resources. It is designed for both local CI checks and live Kubernetes/KubeVirt clusters.

## Why this is different from `zorvia dev diff`

`zorvia dev diff` compares two local configuration files. Drift Guard is operational: it canonicalizes Zorvia `VMConfig` into KubeVirt, removes controller-managed Kubernetes noise, normalizes named arrays, assigns change severity, and can compare directly with the live VM returned by the Kubernetes API.

## Live cluster check

```bash
zorvia change drift vm.yaml
```

The VM name and namespace are inferred from the desired manifest. Override the VM name when needed:

```bash
zorvia --namespace production drift vm.yaml --vm payments-db
```

A non-default kubeconfig supplied through the existing global `--kubeconfig` option is honored.

## File-to-file / CI check

```bash
zorvia change drift desired.yaml --actual captured-live.yaml --fail-on medium
```

Supported output formats:

```bash
zorvia change drift desired.yaml --actual live.yaml --output table
zorvia change drift desired.yaml --actual live.yaml --output json
zorvia change drift desired.yaml --actual live.yaml --output yaml
```

## CI gate

`--fail-on` accepts:

- `none` — report only; never fail because of drift
- `info`
- `low`
- `medium`
- `high` (default)
- `critical`

Example:

```bash
zorvia change drift desired.yaml --actual live.yaml --fail-on high --output json > drift.json
```

When a finding meets or exceeds the threshold, Zorvia returns a non-zero status after rendering the report.

## Ignore paths

Use repeatable RFC 6901-style JSON pointers:

```bash
zorvia change drift desired.yaml \
  --ignore /metadata/labels/build-id \
  --ignore '/metadata/annotations/*'
```

A pointer ending in `/*` suppresses the full subtree.

## Default normalization

The engine ignores controller-owned fields that normally cause false drift:

- `metadata.creationTimestamp`
- `metadata.deletionGracePeriodSeconds`
- `metadata.deletionTimestamp`
- `metadata.generation`
- `metadata.managedFields`
- `metadata.resourceVersion`
- `metadata.selfLink`
- `metadata.uid`
- kubectl last-applied annotation
- KubeVirt observed-version annotations
- `status`

Use `--include-status` when status comparison is intentionally required.

Named object arrays are sorted by their `name` field before comparison. This prevents ordering-only differences in disks, volumes, interfaces, networks and similar KubeVirt lists from being reported as drift.

## Risk model

The first version deliberately uses an explainable deterministic risk model:

| Severity | Typical paths | Rationale |
|---|---|---|
| Critical | disks, volumes, DataVolume templates | storage/source changes can be destructive |
| High | firmware, machine type, network topology, placement | may require restart/migration or change connectivity |
| Medium | CPU, memory, resource requests, running/runStrategy | changes runtime capacity/lifecycle |
| Low | labels and annotations | usually metadata-only |
| Info | other declarative fields | visible but normally lower operational risk |

Risk score is capped at 100 and is intended as a prioritization signal, not a safety guarantee.

## Examples

See `fixtures/drift-desired.yaml` and `fixtures/drift-actual.yaml` in the PR package.
