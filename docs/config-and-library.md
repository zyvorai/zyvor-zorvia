# Config and library

Validate or generate a VM config from the CLI, or build one in Rust.

[Back to the README](../README.md) · [Docs index](README.md)

Zorvia is a library, not just a binary — validate or generate a VM config
from the CLI, or build one programmatically in Rust:

```bash
zorvia template validate examples/ubuntu-cloud-init.yaml
zorvia vm create my-vm --from-file examples/basic-vm.yaml
zorvia template generate web --template ubuntu --kubevirt -o web.yaml
zorvia config config-init && zorvia config config-show
# ~/.config/zorvia/config.toml  ·  /etc/zorvia/config.toml
```

```yaml
name: my-vm
namespace: default
cpu: { cores: 4, sockets: 1, threads: 1 }
memory: { size: 8Gi }
disks:
  - name: rootdisk
    size: 40Gi
    boot_order: 1
    source: { type: Blank }
interfaces:
  - name: default
    network: default
    model: virtio
    network_type: Pod
```

```rust
use zorvia::config::VMConfigBuilder;
use zorvia::output::to_yaml;

fn main() -> anyhow::Result<()> {
    let cfg = VMConfigBuilder::new("my-vm")
        .namespace("production")
        .cpu(4, 1, 1)
        .memory("8Gi")
        .add_blank_disk("rootdisk", "40Gi", 1)
        .add_pod_network("default")
        .label("app", "webserver")
        .build();
    println!("{}", to_yaml(&cfg)?);
    Ok(())
}
```
