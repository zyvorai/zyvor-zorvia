# Golden image fixtures

Generate CDI `DataVolume` + `DataSource` manifests for major Linux images from
`quay.io/containerdisks` (see [docs/GOLDEN_IMAGES.md](../../docs/GOLDEN_IMAGES.md)).

```bash
# Build CLI if needed
cargo build --release
export ZORVIA=./target/release/zorvia

STORAGE_CLASS=<your-sc> ./fixtures/golden-images/generate-bundles.sh
kubectl create ns vm-images --dry-run=client -o yaml | kubectl apply -f -
kubectl apply -f fixtures/golden-images/out/
```

Optional env:

| Variable | Default | Meaning |
|----------|---------|---------|
| `STORAGE_CLASS` | (required) | Cluster StorageClass |
| `NAMESPACE` | `vm-images` | Target namespace |
| `SIZE` | `40Gi` | PVC size (non-Alpine) |
| `OUT_DIR` | `fixtures/golden-images/out` | Output directory |
| `ZORVIA` | `zorvia` | CLI binary path |

Generated YAML under `out/` is gitignored so each lab can pick its StorageClass.
A checked-in sample is [`ubuntu-golden.example.yaml`](ubuntu-golden.example.yaml).
