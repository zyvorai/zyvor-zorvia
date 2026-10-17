# Leaving Nutanix for KubeVirt

A Nutanix VM can be exported through the Prism v4 API with Transiva, converted with h2kvm, and landed on KubeVirt, where Zorvia manages it like any other VM.

> **What was run.** The steps below ran on a single-node k3s cluster with KubeVirt v1.9.1 and Zorvia 0.5.0, against Chimera's Nutanix simulator, not a real Nutanix cluster. The guest was a stock Ubuntu 24.04 cloud image. See [the case study](https://github.com/zyvorai/zyvor-transiva/blob/main/docs/case-study-nutanix-exit-linux.md) for the full list of limits.

## The path

```text
Nutanix (Prism v4)  ->  hyperexport  ->  h2kvm  ->  KubeVirt  ->  Zorvia
```

1. **Export.** `hyperexport --provider nutanix -vm <name> -resolve-containers -mounts <container>:<nfs-path>` writes a qcow2 and `artifact-manifest.json`. The source VM is not changed.
2. **Convert.** `h2kvmctl --cmd local --vmdk <disk> --output-dir <dir> --out-format qcow2 --flatten --fstab-mode stabilize-all --regen-initramfs --checksum` produces the disk to boot.
3. **Land.** Wrap the qcow2 in a containerDisk and create a `VirtualMachine`:

```dockerfile
FROM scratch
ADD --chown=107:107 disk.qcow2 /disk/
```

```yaml
apiVersion: kubevirt.io/v1
kind: VirtualMachine
metadata: {name: migrated-vm, labels: {migrated-from: nutanix}}
spec:
  runStrategy: Always
  template:
    spec:
      domain:
        cpu: {cores: 1}
        resources: {requests: {memory: 512Mi}}
        devices:
          disks: [{name: root, disk: {bus: virtio}}]
          interfaces: [{name: default, masquerade: {}}]
      networks: [{name: default, pod: {}}]
      volumes:
      - name: root
        containerDisk: {image: localhost/migrated-vm:v1, imagePullPolicy: Never}
```

4. **Manage.** Zorvia lists the VM under `/api/v1/vms` as Running, and the usual lifecycle, console and snapshot features apply.

## Measured

One unattended run (`scripts/e2e-nutanix-exit.sh` in Transiva, Ubuntu 24.04 cloud image): 7 of 7 stages passed in 747 s. On KubeVirt the guest was Ready and answering ping 335 s after the disk was converted, including the containerDisk build and import. The same run booted the converted disk on libvirt/KVM (UEFI) in 79 s.

## Not covered

- A real Nutanix cluster, and Windows, UEFI, multi-disk or large VMs.
- Boot repair on a guest that needs it: the Ubuntu image needed only h2kvm's DHCP network fallback.
- Zorvia listing the VM was checked by hand; the script does not check it yet.
- Checks inside the guest beyond ping.

Related: [leave-openshift.md](leave-openshift.md), [DISK_MANAGEMENT.md](DISK_MANAGEMENT.md).
