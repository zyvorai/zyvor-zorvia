# Managed operations

> **Status: Beta, records only.** Zorvia records enrollment, maintenance windows, approved tasks, recovery evidence, incidents, reports and remediation policies. **Nothing here connects to or acts on your cluster.** Remote operation is a recorded, revocable authorization, off by default. No automated remediation engine exists.

## Principles

1. **A contract grants no cluster access.** Buying Managed does not let anyone reach your cluster.
2. **Explicit enrollment.** A member of *your* organization enrolls each cluster. The service desk cannot enroll on your behalf. A cluster is enrollable only while an active Managed contract covers it.
3. **Remote operation is a separate switch.** It is off unless the server sets `ZORVIA_MANAGED_REMOTE_ENABLED=1`, and even then only a member of your organization can enable it, for a stated scope (`monitoring_read`, `maintenance_tasks`, `remediation_actions`) and a *reference* to a credential you issue. The credential itself is never stored. Either side can revoke it at any time.
4. **Automated remediation needs all of:** a documented policy (what it does, when, and the exact permissions), your authorization of that policy, and an audit trail. The `remediation_actions` scope cannot be enabled without an authorized policy. Revoking a policy is immediate.
5. **Expiry never affects your VMs.** When the Managed contract lapses, the enrollment shows `service_eligible: false` and remote operation stops being `effective`. Records are kept; workloads and data are untouched.

## What is recorded

| Record | Who adds it |
|---|---|
| Enrollment: covered components, maintenance windows, monitoring signals | Customer |
| Operational owner | Service desk |
| Maintenance tasks: proposed by the desk, **approved by the customer** before they can be scheduled or completed | Desk proposes, customer approves |
| Backup and restore-test evidence (pass/fail, when, reference) | Customer or desk |
| Incidents (open, mitigated, resolved) and service reports | Service desk |
| Remediation policies: drafted by the desk, authorized or revoked by the customer | Desk drafts, customer authorizes |
| Full event log for every change above | Automatic |

## API

Customer (org-scoped): `GET/POST /api/v1/managed/enrollments`, `GET …/{id}`, `POST …/{id}/remote-operation`, `…/withdraw`, `…/evidence`, `…/tasks/{tid}/approve`, `…/policies/{pid}/authorize|revoke`.

Service desk (`users.admin`): `POST /api/v1/managed/admin/enrollments/{id}/owner|tasks|incidents|reports|policies`, `…/tasks/{tid}/status`, `…/incidents/{iid}`.

See [COMMERCIAL_OFFERINGS.md](COMMERCIAL_OFFERINGS.md) for isolation rules, storage and rollback.
