# Pre-drain maintenance plan

`GET /api/v1/maintenance/plan?node=<node>` (cluster.admin) answers "what happens to my VMs if I
drain this node?" before anyone drains it. It is read-only: it never cordons, drains or migrates.

For every running VM on the node it reports one of:

| action | meaning | disruption |
|---|---|---|
| `live-migrate` | can move, with the destination node chosen | none |
| `downtime` | cannot live-migrate; a drain stops it | downtime |
| `no-destination` | migratable, but no other schedulable node has room | downtime |

with the reasons, plus `can_drain_without_downtime` and warnings (single-node cluster, every other
node cordoned, VMs that can migrate but whose `evictionStrategy` will not do it on drain).

What it looks at:

- KubeVirt's own `LiveMigratable` condition on each VMI, and its message when false;
- volumes that only offer ReadWriteOnce (a volume cannot be attached to two nodes);
- GPUs, host devices and SR-IOV interfaces, which pin a VM to its node;
- the VMI's `evictionStrategy`;
- destination room: allocatable CPU/memory of each schedulable node minus the requests of the VMs
  already on it. VMs are placed largest first onto the least-loaded node that fits, and room is
  reserved as it is promised so two VMs are not offered the same space.

What it does **not** model: storage topology and zone constraints, NUMA and hugepages, node
affinity and taints of the VM, network attachments available on the destination, or pods that
are not VMs. "live-migrate" is a strong hint, not a guarantee; KubeVirt makes the final decision.

```bash
curl -sk -H "Authorization: Bearer $TOKEN" \
  "https://<host>:30152/api/v1/maintenance/plan?node=worker-2" | jq
```
