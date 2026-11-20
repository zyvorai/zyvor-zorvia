# Zorvia OS Templates - Complete Catalog

Zorvia includes **43 named OS templates** (including aliases) across **15 operating system families**.

## Template Statistics

- **Total named keys**: 43
- **OS Families**: 15
- **Linux distro keys**: ~35 (including version aliases)
- **Windows keys**: 5
- **BSD keys**: 3

## 🐧 Available OS Templates

### Ubuntu (4 templates)
```bash
ubuntu          # Default alias → 26.04 LTS
ubuntu-26.04    # Latest LTS (2 CPU, 4GB RAM)
ubuntu-24.04    # Noble Numbat (2 CPU, 4GB RAM)
ubuntu-22.04    # Jammy Jellyfish (2 CPU, 4GB RAM)
```
**Credentials**: `ubuntu` / `ubuntu`

### Fedora (3 templates)
```bash
fedora          # Latest (44)
fedora-44       # Latest stable (2 CPU, 4GB RAM)
fedora-43       # Previous release (2 CPU, 4GB RAM)
```
**Credentials**: `zorvia` / `zorvia`

### CentOS Stream (3 templates)
```bash
centos              # Latest (Stream 10)
centos-stream-10    # Current (2 CPU, 4GB RAM)
centos-stream-9     # Previous version (2 CPU, 4GB RAM)
```
**Credentials**: `zorvia` / `zorvia`

### Debian (3 templates)
```bash
debian          # Latest (13 Trixie)
debian-13       # Trixie (2 CPU, 4GB RAM)
debian-12       # Bookworm (2 CPU, 4GB RAM)
```
**Credentials**: `zorvia` / `zorvia`

### Red Hat Enterprise Linux (3 templates)
```bash
rhel            # Latest (9)
rhel-9          # RHEL 9 (2 CPU, 4GB RAM, 30GB disk)
rhel-8          # RHEL 8 (2 CPU, 4GB RAM, 30GB disk)
```
**Note**: Requires valid RHEL subscription
**Credentials**: `zorvia` / `zorvia`

### AlmaLinux (3 templates)
```bash
almalinux       # Latest (10) - RHEL clone
almalinux-10    # AlmaLinux 10 (2 CPU, 4GB RAM)
almalinux-9     # AlmaLinux 9 (2 CPU, 4GB RAM)
```
**Credentials**: `zorvia` / `zorvia`
**Why**: Free RHEL alternative, binary compatible

### openSUSE (4 templates)
```bash
opensuse                # Latest (Leap 16.0)
opensuse-leap-16.0      # Leap 16.0 (2 CPU, 4GB RAM)
opensuse-leap-15.6      # Leap 15.6 (2 CPU, 4GB RAM)
opensuse-tumbleweed     # Tumbleweed - Rolling (2 CPU, 4GB RAM)
```
**Credentials**: `zorvia` / `zorvia`
**Why**: Enterprise-grade with rolling release option

> **Removed**: Rocky Linux and Alpine templates (and AlmaLinux 8, Ubuntu 18.04/20.04,
> Fedora 39–41, CentOS Stream 8, Debian 11 as end-of-life). `quay.io/containerdisks`
> publishes no `rockylinux` or `alpine` image, so those VMs could only sit in
> `ImagePullBackOff`. Every image above was checked to exist on 2026-09-30.

### Oracle Linux (3 templates) 🆕
```bash
oracle          # Latest (9)
oracle-9        # Oracle Linux 9 (2 CPU, 4GB RAM)
oracle-8        # Oracle Linux 8 (2 CPU, 4GB RAM)
```
**Credentials**: `zorvia` / `zorvia`
**Why**: Oracle's enterprise Linux, RHEL compatible

### Arch Linux (1 template) 🆕
```bash
arch            # Rolling release (2 CPU, 2GB RAM)
```
**Credentials**: `zorvia` / `zorvia`
**Why**: Bleeding edge, highly customizable

### Windows (5 templates)
```bash
windows         # Latest (Server 2022)
windows-2022    # Windows Server 2022 (4 CPU, 8GB RAM, 60GB disk)
windows-2019    # Windows Server 2019 (4 CPU, 8GB RAM, 60GB disk)
windows-11      # Windows 11 (4 CPU dual socket, 8GB RAM, 80GB disk)
windows-10      # Windows 10 (4 CPU, 8GB RAM, 60GB disk)
```
**Note**: Requires Windows license and installation media

### FreeBSD (3 templates) 🆕
```bash
freebsd         # Latest (14)
freebsd-14      # FreeBSD 14 (2 CPU, 2GB RAM)
freebsd-13      # FreeBSD 13 (2 CPU, 2GB RAM)
```
**Why**: BSD Unix, ZFS, jails, high performance

### Flatcar Linux (1 template) 🆕
```bash
flatcar         # Stable - Container-optimized (2 CPU, 2GB RAM)
```
**Credentials**: `zorvia` / `zorvia`
**Why**: Container-optimized, auto-updating, minimal

### Talos Linux (1 template) 🆕
```bash
talos           # Latest - Kubernetes-native (2 CPU, 4GB RAM)
```
**Why**: Purpose-built for Kubernetes, API-driven, no SSH

## 🚀 Quick Usage Examples

### Create VMs with Different OS:

```bash
# Ubuntu 24.04 (latest)
zorvia vm create my-ubuntu --template ubuntu-24.04 --cpus 4 --memory 8Gi

# Fedora 41 (latest)
zorvia vm create my-fedora --template fedora-41 --cpus 2 --memory 4Gi

# AlmaLinux (RHEL alternative)
zorvia vm create my-alma --template almalinux --cpus 4 --memory 16Gi

# Rocky Linux (another RHEL alternative)
zorvia vm create my-rocky --template rocky --cpus 4 --memory 16Gi

# Alpine (ultra lightweight!)
zorvia vm create tiny-vm --template alpine --cpus 1 --memory 512Mi

# Arch Linux (bleeding edge)
zorvia vm create arch-vm --template arch --cpus 2 --memory 2Gi

# OpenSUSE Tumbleweed (rolling)
zorvia vm create suse-vm --template opensuse-tumbleweed

# FreeBSD (Unix)
zorvia vm create bsd-vm --template freebsd-14

# Flatcar (container-optimized)
zorvia vm create flatcar-vm --template flatcar

# Talos (Kubernetes-native)
zorvia vm create k8s-node --template talos --cpus 4 --memory 8Gi

# Windows Server 2022
zorvia vm create win-server --template windows-2022 --cpus 8 --memory 16Gi

# Windows 11
zorvia vm create win11-vm --template windows-11 --cpus 4 --memory 16Gi
```

### View Template Details:

```bash
# Show template configuration
zorvia template show almalinux

# Show as JSON
zorvia template show rocky --output json

# Show as YAML
zorvia template show alpine --output yaml
```

### List All Templates:

```bash
# Simple list
zorvia template templates

# Detailed / JSON output available on `zorvia template show <name>`
zorvia template show ubuntu --output json
```

## 📋 Template Specifications

| Template | CPU | Memory | Disk | User/Pass | Use Case |
|----------|-----|--------|------|-----------|----------|
| ubuntu-24.04 | 2 | 4Gi | 20Gi | ubuntu/ubuntu | General purpose, latest |
| alpine | 1 | 512Mi | 10Gi | alpine/alpine | Microservices, minimal |
| almalinux | 2 | 4Gi | 20Gi | zorvia/zorvia | RHEL alternative, free |
| rocky | 2 | 4Gi | 20Gi | zorvia/zorvia | RHEL alternative, enterprise |
| fedora-41 | 2 | 4Gi | 20Gi | zorvia/zorvia | Latest features |
| arch | 2 | 2Gi | 20Gi | zorvia/zorvia | Bleeding edge |
| opensuse-leap | 2 | 4Gi | 20Gi | zorvia/zorvia | Enterprise stability |
| freebsd-14 | 2 | 2Gi | 20Gi | - | BSD Unix, ZFS |
| flatcar | 2 | 2Gi | 20Gi | zorvia/zorvia | Containers |
| talos | 2 | 4Gi | 20Gi | - | Kubernetes nodes |
| windows-2022 | 4 | 8Gi | 60Gi | - | Windows Server |
| windows-11 | 4 | 8Gi | 80Gi | - | Windows Desktop |

## 🎯 Use Case Guide

### For Web Servers:
```bash
ubuntu-24.04    # Modern, well-supported
rocky           # Enterprise, RHEL-compatible
debian-12       # Stable, lightweight
```

### For Containers:
```bash
alpine          # Minimal footprint
flatcar         # Container-optimized
talos           # Kubernetes-native
```

### For Kubernetes Nodes:
```bash
ubuntu-22.04    # Most common
almalinux       # RHEL compatibility
talos           # Purpose-built for K8s
```

### For Enterprise:
```bash
rhel-9          # Official RHEL
rocky           # Free RHEL alternative
almalinux       # Free RHEL alternative
opensuse-leap   # Enterprise-grade
oracle          # Oracle ecosystem
```

### For Development:
```bash
fedora-41       # Latest packages
arch            # Bleeding edge
ubuntu-24.04    # Modern tools
```

### For Learning/Testing:
```bash
alpine          # Fast, minimal
debian-12       # Stable, well-documented
ubuntu-22.04    # Large community
```

### For BSD Users:
```bash
freebsd-14      # Latest FreeBSD
freebsd-13      # Stable FreeBSD
```

### For Windows:
```bash
windows-2022    # Latest server
windows-11      # Modern desktop
windows-2019    # Stable server
```

## 🔧 Container Disk Images

Most templates use **containerdisks** from `quay.io/containerdisks/`:

- `ubuntu:22.04`, `ubuntu:20.04`, etc.
- `fedora:41`, `fedora:40`, etc.
- `centos-stream:9`, `centos-stream:8`
- `debian:12`, `debian:11`
- `almalinux:9`, `almalinux:8`
- `rockylinux:9`, `rockylinux:8`
- `alpine:3.19`

Some require blank disks (bring your own ISO):
- RHEL (requires subscription)
- Oracle Linux
- Windows (requires license)
- FreeBSD
- OpenSUSE
- Arch Linux

## ✨ Default Credentials Summary

| OS Family | Username | Password |
|-----------|----------|----------|
| Ubuntu | ubuntu | ubuntu |
| Fedora/CentOS/Debian | zorvia | zorvia |
| AlmaLinux/Rocky | zorvia | zorvia |
| Alpine | alpine | alpine |
| Arch/OpenSUSE | zorvia | zorvia |
| Oracle/Flatcar | zorvia | zorvia |
| RHEL | zorvia | zorvia |
| Windows | - | (Configure manually) |
| FreeBSD | - | (Configure manually) |
| Talos | - | (API-driven, no SSH) |

## 🧪 Testing

All templates are tested:

```bash
# Run template tests
cargo test --lib templates

# Test specific template
zorvia template almalinux --output json | jq
zorvia template alpine --output yaml
```

## 📊 Statistics

```
Total named keys:   43
Linux keys:         ~35 (incl. aliases)
Windows:            5
BSD:                3
Container-opt:      2 (Alpine, Flatcar)
Kubernetes-native:  1 (Talos)
Rolling Release:    2 (Arch, OpenSUSE Tumbleweed)
```

## Themed Output

All template commands support the Zorvia theme:

```bash
# Colored template list
zorvia template templates

# Themed template details
zorvia template ubuntu
```

## Future Enhancements

Possible later work:
- [ ] More OS families (Gentoo, NixOS, etc.)
- [ ] Custom template creation UX
- [ ] Template variants (minimal, desktop, server)
- [ ] Template tags and search
- [ ] Community template repository

---

**43 named templates** — pick an OS for the workload and deploy in seconds.
