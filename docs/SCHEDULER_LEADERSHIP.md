# Scheduler leadership safety

Set `ZORVIA_LEADER_ELECTION=1` when using Kubernetes Lease election. An enabled
election starts as a follower. Kubeconfig initialization errors, permission
failures, conflicts, renewal failures and timeouts never grant leadership.
Initialization is retried every five seconds.

Lease requests are bounded to five seconds. A successful renewal grants local
authority only until a monotonic deadline beginning before the request, with a
one-second margin inside the 15-second lease. `is_leader()` checks this deadline,
so a stalled renewal task cannot leave scheduler admission enabled indefinitely.
Foreign holders are displaced only after their own declared lease duration;
missing/invalid foreign timing data blocks takeover. Acquisition patches carry
the observed resourceVersion, and each holder transition resets acquireTime.

The fallback identity is a UUID when `POD_NAME` is absent. Do not deliberately
reuse a `POD_NAME` identity between concurrently running processes. Disabled
election is still supported for a single replica; it grants local leadership
when initialized and must not be used with multiple schedulers.

This prevents new scheduler admission without a valid local lease. It does not
fence handlers or Jobs already running when leadership is lost, provide a
distributed database, or guarantee exactly-once operations. See `docs/HA.md`
for persistence and deployment limitations. Cluster-wide operation ownership
and idempotency still need their own controls.

Validation covers foreign lease duration, exact expiry boundaries, malformed
timing records and local deadline expiry. Exercise API outage, denied RBAC,
pod restart and leadership handover on a real cluster before production rollout.
