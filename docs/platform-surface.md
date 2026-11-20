# Platform surface

Advanced operator surfaces: security, cost, tenancy, backup and DR, HA, automation, observability.

[Back to the README](../README.md) · [Docs index](README.md)

The quickstart above is the everyday path. Underneath it, Zorvia's CLI
carries roughly **178 subcommands** across operator-grade surfaces most teams
grow into over time — security, cost, multi-tenancy, backup/DR, HA,
automation, observability. Per [DEVELOPMENT.md](../DEVELOPMENT.md)'s own scope
note, treat these as **advanced surfaces, not every-cluster guarantees**: the
always-supported path is create → day-2 → snapshots → drift → plan → golden
images → Terraform. Everything below is real and shipped — not a promise
that every module fits every deployment.

**Security & compliance**

```bash
zorvia security security-scan prod-db --scan-type deep
zorvia security security-harden prod-db --profile cis
zorvia security compliance-check prod-db --framework soc2
zorvia security audit-list --security-only
```

**Cost & FinOps**

```bash
zorvia cost cost-analyze prod-db --period 30d
zorvia cost cost-optimize --high-priority-only
zorvia cost cost-waste --waste-type idle
zorvia cost budget-create platform --amount 5000 --period monthly --alert-threshold 80
```

**Multi-tenancy & access**

```bash
zorvia tenancy tenants-create platform-team --owner alice --email alice@example.com
zorvia tenancy users-create bob --email bob@example.com --role operator
zorvia tenancy users-assign-role bob db-admin --scope namespace:staging
zorvia tenancy quotas-create platform-quota --namespace platform --preset large
```

**Backup & disaster recovery**

```bash
zorvia backup backup-create prod-db --backup-type incremental
zorvia backup backup-schedule-create nightly --schedule daily --vm prod-db
zorvia backup backup-restore prod-db-full-20260901 --target prod-db-restore --start
zorvia backup recovery-plan primary-site-failover
```

**High availability**

```bash
zorvia ha ha-config prod-db --enable --priority critical --eviction-strategy live-migrate
zorvia ha ha-status prod-db
zorvia ha evacuate-node worker-3 --max-parallel 4 --plan
```

**Automation & workflows**

```bash
zorvia automation automation-create nightly-snapshot --trigger schedule --enable
zorvia automation workflow-create dr-failover --template disaster-recovery
zorvia automation workflow-run dr-failover --watch
zorvia automation schedule-create weekly-backup --rule nightly-snapshot --schedule weekly --enable
```

**Observability & insights**

```bash
zorvia observability metrics-query cpu_usage --aggregation p95
zorvia observability alerts-active --severity critical
zorvia observability insights-generate prod-db --insight-type performance
zorvia observability trends-analyze cpu_usage --window 24
```
