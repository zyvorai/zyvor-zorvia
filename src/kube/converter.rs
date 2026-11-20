// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

use crate::config::{
    BootloaderType, ClockConfig, DiskDeviceType, DiskSource, FeaturesConfig, FirmwareConfig,
    HostDeviceKind, NetworkType, VMConfig,
};
use crate::kube::types::*;
use anyhow::Result;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use std::collections::BTreeMap;

/// Convert a [`VMConfig`] to a KubeVirt `VirtualMachine` custom resource.
///
/// ```
/// use zorvia::{VMConfigBuilder, vm_config_to_kubevirt};
///
/// let config = VMConfigBuilder::new("my-vm")
///     .namespace("default")
///     .cpu(2, 1, 1)
///     .memory("4Gi")
///     .add_blank_disk("root", "20Gi", 1)
///     .add_pod_network("eth0")
///     .build();
///
/// let vm = vm_config_to_kubevirt(&config).unwrap();
/// assert_eq!(vm.spec.running, Some(false));
/// ```
pub fn vm_config_to_kubevirt(config: &VMConfig) -> Result<VirtualMachine> {
    let mut labels: BTreeMap<String, String> = config.labels.clone().into_iter().collect();
    labels.insert("kubevirt.io/vm".to_string(), config.name.clone());

    // Reuse labels for template metadata (includes kubevirt.io/vm for consistency)
    let template_labels = labels.clone();

    let annotations: BTreeMap<String, String> = config.annotations.clone().into_iter().collect();

    let metadata = ObjectMeta {
        name: Some(config.name.clone()),
        namespace: Some(config.namespace.clone()),
        labels: Some(labels),
        annotations: if annotations.is_empty() {
            None
        } else {
            Some(annotations)
        },
        ..Default::default()
    };

    // Build volumes
    let mut volumes = Vec::new();

    for disk in &config.disks {
        let volume = match &disk.source {
            DiskSource::Blank => Volume {
                name: disk.name.clone(),
                empty_disk: Some(EmptyDiskSource {
                    capacity: disk.size.clone(),
                }),
                container_disk: None,
                persistent_volume_claim: None,
                data_volume: None,
                cloud_init_no_cloud: None,
            },
            DiskSource::PVC { name } => Volume {
                name: disk.name.clone(),
                persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource {
                    claim_name: name.clone(),
                }),
                empty_disk: None,
                container_disk: None,
                data_volume: None,
                cloud_init_no_cloud: None,
            },
            DiskSource::ContainerDisk { image } => Volume {
                name: disk.name.clone(),
                container_disk: Some(ContainerDiskSource {
                    image: image.clone(),
                    image_pull_policy: Some("IfNotPresent".to_string()),
                }),
                empty_disk: None,
                persistent_volume_claim: None,
                data_volume: None,
                cloud_init_no_cloud: None,
            },
            DiskSource::DataVolume { name } => Volume {
                name: disk.name.clone(),
                data_volume: Some(DataVolumeSource { name: name.clone() }),
                empty_disk: None,
                container_disk: None,
                persistent_volume_claim: None,
                cloud_init_no_cloud: None,
            },
        };
        volumes.push(volume);
    }

    // Add cloud-init volume if present
    if let Some(cloud_init) = &config.cloud_init {
        volumes.push(Volume {
            name: "cloudinitdisk".to_string(),
            cloud_init_no_cloud: Some(CloudInitNoCloudSource {
                user_data: Some(cloud_init.user_data.clone()),
                network_data: cloud_init.network_data.clone(),
            }),
            empty_disk: None,
            container_disk: None,
            persistent_volume_claim: None,
            data_volume: None,
        });
    }

    // Build disks
    let mut disks = Vec::new();
    for disk in &config.disks {
        let boot_order = if disk.boot_order > 0 {
            Some(disk.boot_order)
        } else {
            None
        };
        let bus = disk.bus.clone().unwrap_or_else(|| "virtio".to_string());

        match disk.device_type {
            DiskDeviceType::CDROM => {
                disks.push(Disk {
                    name: disk.name.clone(),
                    disk: None,
                    lun: None,
                    cdrom: Some(CDROMTarget {
                        bus: Some(disk.bus.clone().unwrap_or_else(|| "sata".to_string())),
                        readonly: Some(true),
                    }),
                    boot_order,
                    cache: disk.cache.clone(),
                    io: disk.io.clone(),
                    dedicated_io_thread: None,
                    serial: None,
                });
            }
            DiskDeviceType::LUN => {
                disks.push(Disk {
                    name: disk.name.clone(),
                    disk: None,
                    lun: Some(LUNTarget {
                        bus: Some(disk.bus.clone().unwrap_or_else(|| "scsi".to_string())),
                        readonly: None,
                    }),
                    cdrom: None,
                    boot_order,
                    cache: disk.cache.clone(),
                    io: disk.io.clone(),
                    dedicated_io_thread: None,
                    serial: None,
                });
            }
            _ => {
                disks.push(Disk {
                    name: disk.name.clone(),
                    disk: Some(DiskTarget {
                        bus: Some(bus),
                        readonly: None,
                    }),
                    lun: None,
                    cdrom: None,
                    boot_order,
                    cache: disk.cache.clone(),
                    io: disk.io.clone(),
                    dedicated_io_thread: None,
                    serial: None,
                });
            }
        }
    }

    // Add cloud-init disk if present
    if config.cloud_init.is_some() {
        disks.push(Disk {
            name: "cloudinitdisk".to_string(),
            disk: Some(DiskTarget {
                bus: Some("virtio".to_string()),
                readonly: None,
            }),
            lun: None,
            cdrom: None,
            boot_order: None,
            cache: None,
            io: None,
            dedicated_io_thread: None,
            serial: None,
        });
    }

    // Build interfaces
    let mut interfaces = Vec::new();
    for iface in &config.interfaces {
        let (masquerade, bridge, sriov) = match &iface.network_type {
            NetworkType::Pod => (Some(BTreeMap::new()), None, None),
            NetworkType::Bridge => (None, Some(BTreeMap::new()), None),
            NetworkType::Multus { .. } => (None, Some(BTreeMap::new()), None),
            NetworkType::SRIOV { .. } => (None, None, Some(BTreeMap::new())),
        };

        interfaces.push(Interface {
            name: iface.name.clone(),
            model: Some(iface.model.clone()),
            mac_address: iface.mac_address.clone(),
            masquerade,
            bridge,
            sriov,
            ports: None,
            boot_order: None,
        });
    }

    // Build networks
    let mut networks = Vec::new();
    for iface in &config.interfaces {
        let network = match &iface.network_type {
            NetworkType::Pod => Network {
                name: iface.name.clone(),
                pod: Some(BTreeMap::new()),
                multus: None,
            },
            NetworkType::Bridge => Network {
                name: iface.name.clone(),
                pod: Some(BTreeMap::new()),
                multus: None,
            },
            NetworkType::Multus { name } => Network {
                name: iface.name.clone(),
                pod: None,
                multus: Some(MultusNetwork {
                    network_name: name.clone(),
                }),
            },
            NetworkType::SRIOV { name } => Network {
                name: iface.name.clone(),
                pod: None,
                multus: Some(MultusNetwork {
                    network_name: name.clone(),
                }),
            },
        };
        networks.push(network);
    }

    // Build resource requests
    let mut requests = BTreeMap::new();
    requests.insert("memory".to_string(), config.memory.size.clone());

    // Build features
    let features = config.features.as_ref().map(convert_features);

    // Build clock
    let clock = config.clock.as_ref().map(convert_clock);

    // Build firmware
    let firmware = config.firmware.as_ref().map(convert_firmware);

    // Build machine type
    let machine = config.machine_type.as_ref().map(|t| Machine {
        machine_type: Some(t.clone()),
    });

    // Build memory with hugepages
    let memory_hugepages = config
        .memory
        .hugepages_page_size
        .as_ref()
        .map(|ps| Hugepages {
            page_size: Some(ps.clone()),
        });

    // Build devices with TPM, RNG, inputs
    let tpm = if config.enable_tpm {
        Some(TPMDevice {})
    } else {
        None
    };
    let rng = if config.enable_rng {
        Some(RNGDevice {})
    } else {
        None
    };
    // Add tablet input for better mouse support
    let inputs = Some(vec![InputDevice {
        input_type: "tablet".to_string(),
        name: "tablet0".to_string(),
        bus: Some("usb".to_string()),
    }]);

    let termination_grace_period = config.termination_grace_period.unwrap_or(30);

    let spec = VirtualMachineSpec {
        running: Some(false), // VMs are created in stopped state
        run_strategy: None,
        template: VirtualMachineInstanceTemplateSpec {
            metadata: Some(ObjectMeta {
                labels: Some(template_labels),
                ..Default::default()
            }),
            spec: VirtualMachineInstanceSpec {
                domain: DomainSpec {
                    resources: ResourceRequirements {
                        requests: Some(requests),
                        limits: None,
                    },
                    cpu: Some(CPU {
                        cores: Some(config.cpu.cores),
                        sockets: Some(config.cpu.sockets),
                        threads: Some(config.cpu.threads),
                        model: config.cpu.model.clone(),
                        dedicated_cpu_placement: config.cpu.dedicated_cpu_placement,
                        isolate_emulator_thread: config.cpu.isolate_emulator_thread,
                        numa: config
                            .cpu
                            .numa_guest_mapping_passthrough
                            .filter(|on| *on)
                            .map(|_| NUMA {
                                guest_mapping_passthrough: Some(NUMAGuestMapping {}),
                            }),
                        realtime: None,
                        max_sockets: config.cpu.max_sockets,
                        features: None,
                    }),
                    memory: Some(Memory {
                        guest: Some(config.memory.size.clone()),
                        hugepages: memory_hugepages,
                        max_guest: config.memory.max_guest.clone(),
                    }),
                    devices: Some(Devices {
                        gpus: {
                            let g: Vec<GpuDevice> = config
                                .host_devices
                                .iter()
                                .filter(|d| d.kind == HostDeviceKind::Gpu)
                                .map(|d| GpuDevice {
                                    name: d.name.clone(),
                                    device_name: d.device_name.clone(),
                                })
                                .collect();
                            (!g.is_empty()).then_some(g)
                        },
                        host_devices: {
                            let h: Vec<HostDevice> = config
                                .host_devices
                                .iter()
                                .filter(|d| d.kind == HostDeviceKind::HostDevice)
                                .map(|d| HostDevice {
                                    name: d.name.clone(),
                                    device_name: d.device_name.clone(),
                                })
                                .collect();
                            (!h.is_empty()).then_some(h)
                        },
                        disks: Some(disks),
                        interfaces: Some(interfaces),
                        tpm,
                        rng,
                        inputs,
                        watchdog: None,
                        autoattach_graphics_device: None,
                        network_interface_multiqueue: None,
                    }),
                    features,
                    clock,
                    firmware,
                    machine,
                },
                volumes: Some(volumes),
                networks: Some(networks),
                termination_grace_period_seconds: Some(termination_grace_period),
                eviction_strategy: config.eviction_strategy.clone(),
                node_selector: None,
            },
        },
    };

    Ok(VirtualMachine {
        metadata,
        spec,
        status: None,
    })
}

fn convert_features(cfg: &FeaturesConfig) -> Features {
    let hyperv = cfg.hyperv.as_ref().map(|hv| {
        let spinlocks = hv.spinlocks.map(|retries| SpinlockRetries {
            spinlocks: Some(retries),
        });

        HyperVFeatures {
            relaxed: Some(FeatureState {
                enabled: Some(hv.relaxed),
            }),
            vapic: Some(FeatureState {
                enabled: Some(hv.vapic),
            }),
            spinlocks,
            vpindex: Some(FeatureState {
                enabled: Some(hv.vpindex),
            }),
            runtime: Some(FeatureState {
                enabled: Some(hv.runtime),
            }),
            synic: Some(FeatureState {
                enabled: Some(hv.synic),
            }),
            stimer: Some(STimer {
                enabled: Some(hv.stimer),
                direct: None,
            }),
            reset: Some(FeatureState {
                enabled: Some(hv.reset),
            }),
            frequencies: Some(FeatureState {
                enabled: Some(hv.frequencies),
            }),
            reenlightenment: Some(FeatureState {
                enabled: Some(hv.reenlightenment),
            }),
            tlbflush: Some(FeatureState {
                enabled: Some(hv.tlbflush),
            }),
            ipi: Some(FeatureState {
                enabled: Some(hv.ipi),
            }),
            evmcs: None,
        }
    });

    let kvm = cfg.kvm_hidden.map(|hidden| KVMFeatures {
        hidden: Some(hidden),
    });

    let smm = cfg.smm.map(|enabled| SMMFeatures {
        enabled: Some(enabled),
    });

    Features {
        acpi: Some(FeatureEnabled {
            enabled: Some(cfg.acpi),
        }),
        apic: if cfg.apic {
            Some(FeatureEnabled {
                enabled: Some(true),
            })
        } else {
            None
        },
        hyperv,
        kvm,
        smm,
    }
}

fn convert_clock(cfg: &ClockConfig) -> Clock {
    let timer = cfg.timers.as_ref().map(|t| ClockTimer {
        hpet: t.hpet_present.map(|present| TimerConfig {
            present: Some(present),
            tick_policy: None,
        }),
        pit: t.pit_tick_policy.as_ref().map(|policy| TimerConfig {
            present: Some(true),
            tick_policy: Some(policy.clone()),
        }),
        rtc: t.rtc_tick_policy.as_ref().map(|policy| RTCTimer {
            present: Some(true),
            tick_policy: Some(policy.clone()),
            track: Some("guest".to_string()),
        }),
        hyperv: t.hyperv_present.map(|present| TimerConfig {
            present: Some(present),
            tick_policy: None,
        }),
        kvm: None,
    });

    let utc = if cfg.utc {
        Some(UTCClock {
            offset_seconds: None,
        })
    } else {
        None
    };

    Clock {
        utc,
        timezone: cfg.timezone.clone(),
        timer,
    }
}

fn convert_firmware(cfg: &FirmwareConfig) -> Firmware {
    let bootloader = match &cfg.bootloader {
        BootloaderType::BIOS => Bootloader {
            bios: Some(BIOSBootloader {
                use_serial: Some(true),
            }),
            efi: None,
        },
        BootloaderType::EFI {
            secure_boot,
            persistent,
        } => Bootloader {
            bios: None,
            efi: Some(EFIBootloader {
                secure_boot: Some(*secure_boot),
                persistent: Some(*persistent),
            }),
        },
    };

    Firmware {
        bootloader: Some(bootloader),
        uuid: None,
        serial: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::VMConfigBuilder;

    #[test]
    fn test_basic_conversion() {
        let config = VMConfigBuilder::new("test-vm")
            .namespace("default")
            .cpu(2, 1, 1)
            .memory("4Gi")
            .add_blank_disk("rootdisk", "20Gi", 1)
            .add_pod_network("default")
            .build();

        let vm = vm_config_to_kubevirt(&config).unwrap();

        assert_eq!(vm.metadata.name, Some("test-vm".to_string()));
        assert_eq!(vm.metadata.namespace, Some("default".to_string()));
        assert_eq!(vm.spec.running, Some(false));

        let vmi_spec = &vm.spec.template.spec;
        assert_eq!(vmi_spec.domain.cpu.as_ref().unwrap().cores, Some(2));
        assert_eq!(
            vmi_spec.domain.memory.as_ref().unwrap().guest,
            Some("4Gi".to_string())
        );
    }

    #[test]
    fn test_container_disk_conversion() {
        let config = VMConfigBuilder::new("fedora-vm")
            .namespace("default")
            .cpu(4, 1, 1)
            .memory("8Gi")
            .add_container_disk("rootdisk", "quay.io/containerdisks/fedora:39", 1)
            .add_pod_network("default")
            .build();

        let vm = vm_config_to_kubevirt(&config).unwrap();
        let volumes = vm.spec.template.spec.volumes.as_ref().unwrap();

        assert_eq!(volumes.len(), 1);
        assert!(volumes[0].container_disk.is_some());
        assert_eq!(
            volumes[0].container_disk.as_ref().unwrap().image,
            "quay.io/containerdisks/fedora:39"
        );
    }

    #[test]
    fn test_advanced_surface_conversion() {
        use crate::config::{
            BootloaderType, DiskConfig, DiskDeviceType, DiskSource, FeaturesConfig, FirmwareConfig,
            HyperVConfig, InterfaceConfig, NetworkType,
        };

        let config = VMConfigBuilder::new("win-vm")
            .namespace("default")
            .cpu(4, 1, 1)
            .cpu_model("host-passthrough")
            .dedicated_cpu_placement(true)
            .numa_guest_mapping_passthrough(true)
            .isolate_emulator_thread(true)
            .memory("8Gi")
            .hugepages("2Mi")
            .machine_type("q35")
            .enable_tpm()
            .enable_rng()
            .firmware(FirmwareConfig {
                bootloader: BootloaderType::EFI {
                    secure_boot: true,
                    persistent: true,
                },
            })
            .features(FeaturesConfig {
                acpi: true,
                apic: true,
                hyperv: Some(HyperVConfig {
                    relaxed: true,
                    vapic: true,
                    spinlocks: Some(8191),
                    vpindex: true,
                    runtime: false,
                    synic: true,
                    stimer: true,
                    reset: false,
                    frequencies: false,
                    reenlightenment: false,
                    tlbflush: true,
                    ipi: true,
                }),
                kvm_hidden: Some(true),
                smm: Some(true),
            })
            .add_disk(DiskConfig {
                name: "rootdisk".into(),
                size: "60Gi".into(),
                storage_class: None,
                boot_order: 1,
                source: DiskSource::Blank,
                device_type: DiskDeviceType::Disk,
                bus: Some("virtio".into()),
                cache: None,
                io: None,
            })
            .add_disk(DiskConfig {
                name: "datadisk".into(),
                size: "100Gi".into(),
                storage_class: Some("fast-ssd".into()),
                boot_order: 2,
                source: DiskSource::Blank,
                device_type: DiskDeviceType::Disk,
                bus: Some("virtio".into()),
                cache: None,
                io: None,
            })
            .add_interface(InterfaceConfig {
                name: "eth0".into(),
                network: "default".into(),
                model: "virtio".into(),
                network_type: NetworkType::Pod,
                mac_address: None,
            })
            .add_interface(InterfaceConfig {
                name: "eth1".into(),
                network: "storage-net".into(),
                model: "virtio".into(),
                network_type: NetworkType::Multus {
                    name: "storage-net".into(),
                },
                mac_address: Some("52:54:00:12:34:56".into()),
            })
            .build();

        let vm = vm_config_to_kubevirt(&config).unwrap();
        let domain = &vm.spec.template.spec.domain;

        let cpu = domain.cpu.as_ref().unwrap();
        assert_eq!(cpu.model, Some("host-passthrough".to_string()));
        assert_eq!(cpu.dedicated_cpu_placement, Some(true));
        assert_eq!(cpu.isolate_emulator_thread, Some(true));
        let numa = serde_json::to_value(cpu.numa.as_ref().unwrap()).unwrap();
        assert_eq!(numa, serde_json::json!({ "guestMappingPassthrough": {} }));

        assert!(domain.memory.as_ref().unwrap().hugepages.is_some());
        assert_eq!(
            domain.machine.as_ref().unwrap().machine_type,
            Some("q35".to_string())
        );

        let devices = domain.devices.as_ref().unwrap();
        assert!(devices.tpm.is_some());
        assert!(devices.rng.is_some());
        assert_eq!(devices.disks.as_ref().unwrap().len(), 2);
        assert_eq!(devices.interfaces.as_ref().unwrap().len(), 2);
        assert_eq!(
            devices.interfaces.as_ref().unwrap()[1].mac_address,
            Some("52:54:00:12:34:56".to_string())
        );

        let firmware = domain.firmware.as_ref().unwrap();
        let bootloader = firmware.bootloader.as_ref().unwrap();
        assert!(bootloader.bios.is_none());
        assert_eq!(bootloader.efi.as_ref().unwrap().secure_boot, Some(true));

        let features = domain.features.as_ref().unwrap();
        assert_eq!(features.acpi.as_ref().unwrap().enabled, Some(true));
        assert!(features.apic.is_some());
        assert!(features.hyperv.is_some());
        assert!(features.smm.is_some());

        let volumes = vm.spec.template.spec.volumes.as_ref().unwrap();
        assert_eq!(volumes.len(), 2);
        let networks = vm.spec.template.spec.networks.as_ref().unwrap();
        assert_eq!(networks.len(), 2);
    }

    #[test]
    fn test_cloud_init_conversion() {
        let config = VMConfigBuilder::new("ubuntu-vm")
            .namespace("default")
            .cpu(2, 1, 1)
            .memory("4Gi")
            .add_blank_disk("rootdisk", "20Gi", 1)
            .add_pod_network("default")
            .cloud_init("#cloud-config\nuser: ubuntu")
            .build();

        let vm = vm_config_to_kubevirt(&config).unwrap();
        let volumes = vm.spec.template.spec.volumes.as_ref().unwrap();

        // Should have rootdisk + cloudinitdisk
        assert_eq!(volumes.len(), 2);

        let cloud_init_vol = volumes.iter().find(|v| v.name == "cloudinitdisk");
        assert!(cloud_init_vol.is_some());
        assert!(cloud_init_vol.unwrap().cloud_init_no_cloud.is_some());
    }

    #[test]
    fn gpus_and_host_devices_become_the_right_kubevirt_fields() {
        let config = VMConfigBuilder::new("gpu-vm")
            .namespace("default")
            .cpu(4, 1, 1)
            .memory("8Gi")
            .add_blank_disk("rootdisk", "20Gi", 1)
            .add_pod_network("default")
            .add_gpu("gpu0", "nvidia.com/GA102GL_A10")
            .add_host_device("nic0", "intel.com/x710")
            .build();
        let vm = vm_config_to_kubevirt(&config).unwrap();
        let dev = vm.spec.template.spec.domain.devices.as_ref().unwrap();
        let gpus = dev.gpus.as_ref().unwrap();
        assert_eq!(
            (gpus[0].name.as_str(), gpus[0].device_name.as_str()),
            ("gpu0", "nvidia.com/GA102GL_A10")
        );
        let hd = dev.host_devices.as_ref().unwrap();
        assert_eq!(
            (hd[0].name.as_str(), hd[0].device_name.as_str()),
            ("nic0", "intel.com/x710")
        );
        // The wire shape KubeVirt expects: camelCase deviceName / hostDevices.
        let json = serde_json::to_value(dev).unwrap();
        assert_eq!(json["gpus"][0]["deviceName"], "nvidia.com/GA102GL_A10");
        assert_eq!(json["hostDevices"][0]["deviceName"], "intel.com/x710");
        // A VM without devices emits neither field.
        let plain = VMConfigBuilder::new("plain")
            .namespace("default")
            .cpu(1, 1, 1)
            .memory("1Gi")
            .add_blank_disk("rootdisk", "5Gi", 1)
            .add_pod_network("default")
            .build();
        let vm = vm_config_to_kubevirt(&plain).unwrap();
        let json =
            serde_json::to_value(vm.spec.template.spec.domain.devices.as_ref().unwrap()).unwrap();
        assert!(json.get("gpus").is_none() && json.get("hostDevices").is_none());
    }

    #[test]
    fn an_sriov_interface_uses_the_sriov_binding_on_a_multus_network() {
        let config = VMConfigBuilder::new("sriov-vm")
            .namespace("default")
            .cpu(2, 1, 1)
            .memory("4Gi")
            .add_blank_disk("rootdisk", "20Gi", 1)
            .add_interface(crate::config::InterfaceConfig {
                name: "vf0".into(),
                network: "vf-net".into(),
                model: "virtio".into(),
                network_type: NetworkType::SRIOV {
                    name: "vf-net".into(),
                },
                mac_address: None,
            })
            .build();
        let vm = vm_config_to_kubevirt(&config).unwrap();
        let json = serde_json::to_value(&vm.spec.template.spec).unwrap();
        assert!(json["domain"]["devices"]["interfaces"][0]
            .get("sriov")
            .is_some());
        assert_eq!(json["networks"][0]["multus"]["networkName"], "vf-net");
    }
}
