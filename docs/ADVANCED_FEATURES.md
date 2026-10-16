# 🚀 Advanced Features Guide

This document covers the advanced features added to **zorvia** in the innovation phase.

---

## 📊 Status Command — Platform + VM

### Platform status (Cilium-style)

Omit the VM name for a colorful cluster summary — interlocking logo, component
OK/disabled lines, workloads, and feature inventory:

```bash
# Platform / cluster status
zorvia vm status
zorvia vm status -o json
zorvia vm status --wait --wait-duration 5m
```

Example (colors + emojis abbreviated):

```text
    /¯¯\
 /¯¯\__/¯¯\    KubeVirt:       ✅ OK
 \__/¯¯\__/    CDI:            ✅ OK
 /¯¯\__/¯¯\    Zorvia API:     ✅ OK
 \__/¯¯\__/    Snapshots:      ℹ️  disabled
    \__/       Rook Storage:   ℹ️  disabled

🚀 Deployment      virt-api        Desired: 1, Ready: 1/1, Available: 1/1
📦 DaemonSet       virt-handler    Desired: 3, Ready: 3/3, Available: 3/3
🖥️  Cluster VMs:    12/15 managed by Zorvia
✨ Features        Backup          ✅ OK
                   Kryton          ℹ️  disabled
```

### Per-VM status

```bash
# Show detailed VM status
zorvia vm status my-vm

# Watch mode - continuously update status
zorvia vm status my-vm --watch

# Custom update interval (in seconds)
zorvia vm status my-vm --watch --interval 5
```

### Example VM output

```
═══════════════════════════════════════════════════════════════
VM Status: web-server-1
═══════════════════════════════════════════════════════════════

Basic Information:
  Name:       web-server-1
  Namespace:  production
  Running:    ✓ Yes
  Ready:      ✓ Yes
  Phase:      Running
  Created:    2024-02-05T10:30:45Z

Resources:
  CPU Cores:  4
```
  Memory:     8Gi

Volumes (2):
  • rootdisk
  • datadisk

Networks (1):
  • default

Conditions:
  ✓ Ready - True
      Reason: VMReady
      Message: VM is ready and running
```

---

## 🔄 Clone Command - Duplicate VMs

Clone an existing VM to create a new one with the same configuration.

### Usage

```bash
# Basic clone
zorvia vm clone source-vm target-vm

# Clone and start immediately
zorvia vm clone source-vm target-vm --start
```

### Example

```bash
# Clone production database for testing
zorvia vm clone prod-db test-db

# Clone and start for development
zorvia vm clone web-server-1 web-server-2 --start
```

### What Gets Cloned

✓ CPU configuration
✓ Memory settings
✓ Disk configurations
✓ Network interfaces
✓ Labels (except kubevirt.io/vm)
✓ Cloud-init data

⚠️ **Note:** Actual disk data is NOT cloned - you get empty disks of the same size.

---

## 📈 Resources Command - Usage Summary

View resource allocation across all VMs with sorting and filtering.

### Usage

```bash
# Show resources in current namespace
zorvia vm resources

# Show resources across all namespaces
zorvia vm resources --all-namespaces

# Sort by CPU usage
zorvia vm resources --sort-by cpu

# Sort by memory
zorvia vm resources --sort-by memory
```

### Example Output

```
╔═══════════════════════════════════════════════════════════════╗
║                   Resource Summary                            ║
╚═══════════════════════════════════════════════════════════════╝

VMs:
  Total:      5
  Running:    3 ✓
  Stopped:    2 ✗

Resources (Requested):
  Total CPU:    24 cores
  Total Memory: 48.00 Gi

Per VM Average:
  CPU:    4.8 cores
  Memory: 9.60 Gi

╔═══════════════════════════════════════════════════════════════╗
║                   VM Resource Details                         ║
╚═══════════════════════════════════════════════════════════════╝

NAME                           NAMESPACE        CPU        MEMORY
----------------------------------------------------------------------
web-server-1                   production       4          8Gi
web-server-2                   production       4          8Gi
database-1                     production       8          16Gi
test-vm                        development      4          8Gi
dev-vm                         development      4          8Gi
```

---

## 📤 Export Command - Save VM Configurations

Export existing VM configurations for backup or migration.

### Usage

```bash
# Export to stdout
zorvia vm export my-vm

# Export to file
zorvia vm export my-vm --output backup.yaml

# Export as KubeVirt manifest
zorvia vm export my-vm --kubevirt --output vm.kubevirt.yaml
```

### Use Cases

1. **Backup:** Save VM configurations before making changes
2. **Migration:** Export from one cluster and import to another
3. **Version Control:** Track VM configurations in git
4. **Templates:** Create custom templates from existing VMs

---

## 🧙 Wizard Command - Interactive VM Creation

Interactive, guided VM creation with prompts and defaults.

### Usage

```bash
# Start wizard without pre-filling name
zorvia vm wizard

# Start wizard with name pre-filled
zorvia vm wizard my-new-vm
```

### Interactive Flow

```
╔═══════════════════════════════════════════════════════════════╗
║           Interactive VM Creation Wizard                      ║
╚═══════════════════════════════════════════════════════════════╝

VM Name: web-server-prod
Select a template:
  > ubuntu
    centos
    fedora
    debian
    rhel
    windows

CPU Cores [2]: 4
Memory (e.g., 4Gi, 8Gi) [4Gi]: 8Gi
Disk Size (e.g., 20Gi, 40Gi) [20Gi]: 100Gi

Start VM immediately?
  > No
    Yes

Creating VM with the following configuration:
  Name:     web-server-prod
  Template: ubuntu
  CPU:      4 cores
  Memory:   8Gi
  Disk:     100Gi

✓ VM 'web-server-prod' created successfully
```

### Benefits

- No need to remember flags or options
- Visual template selection
- Sensible defaults
- Validation before creation
- Great for beginners

---

## 🎯 Batch Command - Create Multiple VMs

Create multiple VMs at once from a batch configuration file.

### Usage

```bash
# Dry run to preview
zorvia vm batch cluster.yaml --dry-run

# Create all VMs
zorvia vm batch cluster.yaml

# Override namespace for all VMs
zorvia vm batch cluster.yaml --namespace production

# Continue on errors instead of stopping
zorvia vm batch cluster.yaml --continue-on-error
```

### Batch Configuration Format

```yaml
namespace: production

vms:
  - name: web-server-1
    namespace: production
    cpu:
      cores: 4
      sockets: 1
      threads: 1
    memory:
      size: 8Gi
    disks:
      - name: rootdisk
        size: 40Gi
        boot_order: 1
        source:
          type: containerDisk
          image: quay.io/containerdisks/ubuntu:22.04
    interfaces:
      - name: default
        network: default
        model: virtio
        network_type: pod
    cloud_init:
      user_data: |
        #cloud-config
        hostname: web-server-1
        packages:
          - envoy
    labels:
      app: web
      tier: frontend

  - name: web-server-2
    # ... similar configuration

  - name: web-server-3
    # ... similar configuration
```

### Example Output

```
Loading batch configuration from: cluster.yaml
Found 3 VMs to create

[00:00:15] ========================================> 3/3 Batch creation complete
  ✓ web-server-1 created successfully
  ✓ web-server-2 created successfully
  ✓ web-server-3 created successfully

╔═══════════════════════════════════════════════════════════════╗
║                   Batch Summary                               ║
╚═══════════════════════════════════════════════════════════════╝
  Total VMs:    3
  Successful:   3 ✓
  Failed:       0 ✗
```

### Use Cases

1. **Cluster Deployment:** Deploy entire application stacks
2. **Testing:** Create multiple test environments
3. **Development:** Spin up dev clusters
4. **Demos:** Quickly create demonstration environments

### Best Practices

- Use `--dry-run` first to verify configuration
- Enable `--continue-on-error` for large batches
- Organize batch files by environment or purpose
- Use meaningful VM names and labels
- Version control your batch configurations

---

## 🎨 Features Comparison

| Feature | Basic CLI | Advanced Features |
|---------|-----------|-------------------|
| Create VM | ✓ | ✓ |
| List VMs | ✓ | ✓ |
| Delete VM | ✓ | ✓ |
| Start/Stop | ✓ | ✓ |
| Generate Manifest | ✓ | ✓ |
| **Detailed Status** | ✗ | ✓ |
| **Clone VM** | ✗ | ✓ |
| **Resource Summary** | ✗ | ✓ |
| **Export Config** | ✗ | ✓ |
| **Interactive Wizard** | ✗ | ✓ |
| **Batch Creation** | ✗ | ✓ |
| **Watch Mode** | ✗ | ✓ |

---

## 🔧 Tips & Tricks

### 1. Resource Planning

Use `resources` command to understand cluster utilization before creating new VMs:

```bash
zorvia vm resources --all-namespaces --sort-by cpu
```

### 2. VM Templating

Export an existing VM as a template:

```bash
zorvia vm export prod-db --output templates/database-template.yaml
# Edit template
# Use with batch command
```

### 3. Quick Cloning for Testing

```bash
# Clone production to staging for testing
zorvia vm clone prod-web staging-web
zorvia vm get staging-web  # Verify
zorvia vm start staging-web  # Test
```

### 4. Monitoring with Watch

Monitor VM startup in real-time:

```bash
zorvia vm status my-vm --watch --interval 2
```

### 5. Infrastructure as Code

Combine batch configurations with git:

```bash
git clone https://github.com/myorg/vm-configs.git
cd vm-configs/production
zorvia vm batch web-cluster.yaml
```

---

## 📊 Performance Considerations

### Batch Operations

- Batch creation is sequential (one VM at a time)
- Use `--continue-on-error` for resilience
- Consider cluster capacity before large batches
- Monitor with `resources` command during creation

### Resource Limits

- Check namespace quotas before batch creation
- Verify storage class availability
- Ensure sufficient node resources

### Best Practices

1. **Start Small:** Test with 1-2 VMs before full batch
2. **Use Dry Run:** Always verify with `--dry-run` first
3. **Monitor Progress:** Use watch mode during creation
4. **Check Resources:** Run `resources` before and after
5. **Validate Configs:** Use `validate` command on configs

---

## 🎯 Real-World Scenarios

### Scenario 1: Web Application Cluster

```bash
# Create load-balanced web cluster
zorvia vm batch examples/batch-web-cluster.yaml

# Monitor resource usage
zorvia vm resources --namespace production

# Check status of all servers
for vm in web-server-{1..3}; do
  zorvia vm status $vm
done
```

### Scenario 2: Development Environment

```bash
# Interactive creation for quick dev VM
zorvia vm wizard

# Clone for teammate
zorvia vm clone my-dev-vm teammate-dev-vm --start

# Export configuration for sharing
zorvia vm export my-dev-vm --output team-dev-config.yaml
```

### Scenario 3: Disaster Recovery

```bash
# Export all VMs for backup
for vm in $(zorvia vm list --output json | jq -r '.[].metadata.name'); do
  zorvia vm export $vm --output backups/$vm.yaml
done

# Restore from backups
zorvia vm batch backups/*.yaml
```

---

## Related day-2 docs

Many items once listed as “next” here now ship elsewhere:

| Capability | Where |
|------------|--------|
| Snapshots & retention | [SNAPSHOTS.md](SNAPSHOTS.md) |
| Health & monitoring | CLI `zorvia vm status` (platform) / `zorvia advisor health` / `zorvia monitor ...`; web metrics |
| Live migration | CLI `zorvia migration migrate` / HA helpers |
| Web console & Fabric API | [WEB_CONSOLE.md](WEB_CONSOLE.md) |
| Drift / plan / guest insight | [DRIFT_GUARD.md](DRIFT_GUARD.md), [CHANGE_PLANNER.md](CHANGE_PLANNER.md), [GUEST_INSIGHT.md](GUEST_INSIGHT.md) |

Near-term product roadmap (registry Terraform provider, etc.) lives in the root [README.md](../README.md) and [CHANGELOG.md](../CHANGELOG.md).

---

See the main [README.md](../README.md) for basic usage, [WEB_CONSOLE.md](WEB_CONSOLE.md) for the web API, and [DEVELOPMENT.md](../DEVELOPMENT.md) for development details.
