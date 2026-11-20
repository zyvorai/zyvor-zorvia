# Profiles, blueprints and templates

Three lookup tables: a shape, a stack, or an OS. No CRD authoring.

[Back to the README](../README.md) · [Docs index](README.md)

Look up a shape, ship a stack, or name an OS — three lookup tables, no CRD
authoring.

**Profiles** — look up a shape, then pass resources on `create`:

| Profile | CPU | Memory | Disk | Best for |
|---------|-----|--------|------|----------|
| minimal | 1 | 512Mi | 5Gi | Agents, jump hosts |
| dev | 1 | 2Gi | 10Gi | Learning |
| test | 2 | 4Gi | 20Gi | CI |
| web | 4 | 8Gi | 40Gi | Envoy, static |
| prod | 4 | 8Gi | 40Gi | Production apps |
| database | 6 | 16Gi | 200Gi | Databases |
| microservice | 2 | 4Gi | 20Gi | Node roles |
| high-perf | 8 | 16Gi | 100Gi | ML / heavy I/O |

```bash
zorvia profile profiles
zorvia profile show web
zorvia vm create api --template fedora-44 --cpus 4 --memory 8Gi --disk-size 40Gi
```

**Blueprints** — multi-VM stacks (profiles applied inside the blueprint):

| Blueprint | VMs | Stack |
|-----------|-----|--------|
| lamp | 2 | MySQL + Apache |
| k8s-cluster | 3 | Control plane + workers |
| 3tier | 3 | DB + app + Envoy |
| cicd | 3 | GitLab + Jenkins + registry |
| dev-stack | 3 | DB + Redis + workspace |

```bash
zorvia blueprint show lamp
zorvia blueprint deploy lamp --prefix demo --dry-run
zorvia blueprint deploy lamp --prefix demo --start
```

**Templates** — 43 named keys (aliases included): Ubuntu, Fedora, CentOS Stream, Debian, RHEL, Alma, Rocky, OpenSUSE, Alpine, Arch, Oracle, FreeBSD, Flatcar, Talos, Windows.

```bash
zorvia template templates
zorvia template show ubuntu-24.04
```

Catalog: [docs/OS_TEMPLATES.md](OS_TEMPLATES.md) · gallery shot above under [Console gallery](../README.md#console-gallery).
