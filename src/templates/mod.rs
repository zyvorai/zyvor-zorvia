// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

use crate::config::*;
use once_cell::sync::Lazy;
use std::collections::HashMap;

// ============================================================================
// Shared feature/firmware/clock presets
// ============================================================================

/// Windows HyperV enlightenments for optimal performance
fn windows_features() -> FeaturesConfig {
    FeaturesConfig {
        acpi: true,
        apic: true,
        hyperv: Some(HyperVConfig {
            relaxed: true,
            vapic: true,
            spinlocks: Some(8191),
            vpindex: true,
            runtime: true,
            synic: true,
            stimer: true,
            reset: true,
            frequencies: true,
            reenlightenment: true,
            tlbflush: true,
            ipi: true,
        }),
        kvm_hidden: None,
        smm: Some(true),
    }
}

/// UEFI firmware with Secure Boot (required for Windows 11)
fn uefi_secure_boot_firmware() -> FirmwareConfig {
    FirmwareConfig {
        bootloader: BootloaderType::EFI {
            secure_boot: true,
            persistent: true,
        },
    }
}

/// UEFI firmware without Secure Boot
fn uefi_firmware() -> FirmwareConfig {
    FirmwareConfig {
        bootloader: BootloaderType::EFI {
            secure_boot: false,
            persistent: true,
        },
    }
}

/// Windows clock configuration with HyperV timer
fn windows_clock() -> ClockConfig {
    ClockConfig {
        utc: true,
        timezone: None,
        timers: Some(TimersConfig {
            hpet_present: Some(false),
            pit_tick_policy: Some("delay".to_string()),
            rtc_tick_policy: Some("catchup".to_string()),
            hyperv_present: Some(true),
        }),
    }
}

/// Linux clock configuration
fn linux_clock() -> ClockConfig {
    ClockConfig {
        utc: true,
        timezone: None,
        timers: Some(TimersConfig {
            hpet_present: Some(false),
            pit_tick_policy: Some("delay".to_string()),
            rtc_tick_policy: Some("catchup".to_string()),
            hyperv_present: None,
        }),
    }
}

pub static TEMPLATES: Lazy<TemplateManager> = Lazy::new(TemplateManager::new);

/// Manages VM templates
pub struct TemplateManager {
    templates: HashMap<String, VMConfig>,
}

impl TemplateManager {
    pub fn new() -> Self {
        let mut templates = HashMap::new();

        // Ubuntu variants (aliases point at the newest LTS)
        templates.insert("ubuntu".to_string(), ubuntu_2604_template());
        templates.insert("ubuntu-26.04".to_string(), ubuntu_2604_template());
        templates.insert("ubuntu-24.04".to_string(), ubuntu_2404_template());
        templates.insert("ubuntu-22.04".to_string(), ubuntu_2204_template());

        // Fedora variants
        templates.insert("fedora".to_string(), fedora_44_template());
        templates.insert("fedora-44".to_string(), fedora_44_template());
        templates.insert("fedora-43".to_string(), fedora_43_template());

        // CentOS Stream variants
        templates.insert("centos".to_string(), centos_stream10_template());
        templates.insert("centos-stream-10".to_string(), centos_stream10_template());
        templates.insert("centos-stream-9".to_string(), centos_stream9_template());

        // Debian variants
        templates.insert("debian".to_string(), debian_13_template());
        templates.insert("debian-13".to_string(), debian_13_template());
        templates.insert("debian-12".to_string(), debian_12_template());

        // RHEL variants
        templates.insert("rhel".to_string(), rhel_9_template());
        templates.insert("rhel-9".to_string(), rhel_9_template());
        templates.insert("rhel-8".to_string(), rhel_8_template());

        // AlmaLinux
        templates.insert("almalinux".to_string(), almalinux_10_template());
        templates.insert("almalinux-10".to_string(), almalinux_10_template());
        templates.insert("almalinux-9".to_string(), almalinux_9_template());

        // openSUSE (Leap releases and rolling Tumbleweed)
        templates.insert("opensuse".to_string(), opensuse_leap_16_template());
        templates.insert("opensuse-leap".to_string(), opensuse_leap_16_template());
        templates.insert(
            "opensuse-leap-16.0".to_string(),
            opensuse_leap_16_template(),
        );
        templates.insert(
            "opensuse-leap-15.6".to_string(),
            opensuse_leap_15_template(),
        );
        templates.insert(
            "opensuse-tumbleweed".to_string(),
            opensuse_tumbleweed_template(),
        );

        // Arch Linux
        templates.insert("arch".to_string(), arch_template());

        // Oracle Linux
        templates.insert("oracle".to_string(), oracle_9_template());
        templates.insert("oracle-9".to_string(), oracle_9_template());
        templates.insert("oracle-8".to_string(), oracle_8_template());

        // Windows variants
        templates.insert("windows".to_string(), windows_2022_template());
        templates.insert("windows-2022".to_string(), windows_2022_template());
        templates.insert("windows-2019".to_string(), windows_2019_template());
        templates.insert("windows-11".to_string(), windows_11_template());
        templates.insert("windows-10".to_string(), windows_10_template());

        // FreeBSD
        templates.insert("freebsd".to_string(), freebsd_14_template());
        templates.insert("freebsd-14".to_string(), freebsd_14_template());
        templates.insert("freebsd-13".to_string(), freebsd_13_template());

        // Flatcar Linux
        templates.insert("flatcar".to_string(), flatcar_template());

        // Talos Linux (for Kubernetes)
        templates.insert("talos".to_string(), talos_template());

        Self { templates }
    }

    pub fn get(&self, name: &str) -> Option<VMConfig> {
        self.templates.get(name).cloned()
    }

    pub fn list(&self) -> Vec<String> {
        let mut names: Vec<_> = self.templates.keys().cloned().collect();
        names.sort();
        names
    }

    pub fn exists(&self, name: &str) -> bool {
        self.templates.contains_key(name)
    }

    /// Get templates grouped by OS family
    pub fn list_by_family(&self) -> HashMap<String, Vec<String>> {
        let mut families: HashMap<String, Vec<String>> = HashMap::new();

        for name in self.templates.keys() {
            let family = if name.starts_with("ubuntu") {
                "Ubuntu"
            } else if name.starts_with("fedora") {
                "Fedora"
            } else if name.starts_with("centos") {
                "CentOS"
            } else if name.starts_with("debian") {
                "Debian"
            } else if name.starts_with("rhel") {
                "RHEL"
            } else if name.starts_with("almalinux") {
                "AlmaLinux"
            } else if name.starts_with("opensuse") {
                "OpenSUSE"
            } else if name.starts_with("arch") {
                "Arch Linux"
            } else if name.starts_with("oracle") {
                "Oracle Linux"
            } else if name.starts_with("windows") {
                "Windows"
            } else if name.starts_with("freebsd") {
                "FreeBSD"
            } else if name.starts_with("flatcar") {
                "Flatcar"
            } else if name.starts_with("talos") {
                "Talos"
            } else {
                "Other"
            };

            families
                .entry(family.to_string())
                .or_default()
                .push(name.clone());
        }

        // Sort each family's templates
        for templates in families.values_mut() {
            templates.sort();
        }

        families
    }
}

impl Default for TemplateManager {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Ubuntu Templates
// ============================================================================

// ============================================================================
// Fedora Templates
// ============================================================================

// ============================================================================
// CentOS Templates
// ============================================================================

// ============================================================================
// Debian Templates
// ============================================================================

// ============================================================================
// RHEL Templates
// ============================================================================

fn rhel_9_template() -> VMConfig {
    VMConfigBuilder::new("rhel-vm")
        .namespace("default")
        .cpu(2, 1, 1)
        .memory("4Gi")
        .add_blank_disk("rootdisk", "30Gi", 1)
        .add_pod_network("default")
        .label("os", "rhel")
        .label("os.version", "9")
        .cloud_init(default_cloud_init())
        .enable_rng()
        .clock(linux_clock())
        .build()
}

fn rhel_8_template() -> VMConfig {
    VMConfigBuilder::new("rhel-vm")
        .namespace("default")
        .cpu(2, 1, 1)
        .memory("4Gi")
        .add_blank_disk("rootdisk", "30Gi", 1)
        .add_pod_network("default")
        .label("os", "rhel")
        .label("os.version", "8")
        .cloud_init(default_cloud_init())
        .enable_rng()
        .clock(linux_clock())
        .build()
}

// ============================================================================
// AlmaLinux Templates
// ============================================================================

// ============================================================================
// Rocky Linux Templates
// ============================================================================

// ============================================================================
// OpenSUSE Templates
// ============================================================================

// ============================================================================
// Alpine Linux Template
// ============================================================================

// ============================================================================
// Arch Linux Template
// ============================================================================

fn arch_template() -> VMConfig {
    VMConfigBuilder::new("arch-vm")
        .namespace("default")
        .cpu(2, 1, 1)
        .memory("2Gi")
        .add_blank_disk("rootdisk", "20Gi", 1)
        .add_pod_network("default")
        .label("os", "arch")
        .label("os.version", "latest")
        .cloud_init(default_cloud_init())
        .enable_rng()
        .clock(linux_clock())
        .build()
}

// ============================================================================
// Oracle Linux Templates
// ============================================================================

fn oracle_9_template() -> VMConfig {
    VMConfigBuilder::new("oracle-vm")
        .namespace("default")
        .cpu(2, 1, 1)
        .memory("4Gi")
        .add_blank_disk("rootdisk", "30Gi", 1)
        .add_pod_network("default")
        .label("os", "oracle")
        .label("os.version", "9")
        .cloud_init(default_cloud_init())
        .enable_rng()
        .clock(linux_clock())
        .build()
}

fn oracle_8_template() -> VMConfig {
    VMConfigBuilder::new("oracle-vm")
        .namespace("default")
        .cpu(2, 1, 1)
        .memory("4Gi")
        .add_blank_disk("rootdisk", "30Gi", 1)
        .add_pod_network("default")
        .label("os", "oracle")
        .label("os.version", "8")
        .cloud_init(default_cloud_init())
        .enable_rng()
        .clock(linux_clock())
        .build()
}

// ============================================================================
// Windows Templates
// ============================================================================

fn windows_2022_template() -> VMConfig {
    VMConfigBuilder::new("windows-vm")
        .namespace("default")
        .cpu(4, 1, 1)
        .memory("8Gi")
        .add_blank_disk("rootdisk", "60Gi", 1)
        .add_cdrom("virtio-drivers", "quay.io/containerdisks/virtio-win", 2)
        .add_pod_network("default")
        .label("os", "windows")
        .label("os.version", "2022")
        .features(windows_features())
        .firmware(uefi_firmware())
        .clock(windows_clock())
        .enable_tpm()
        .enable_rng()
        .termination_grace_period(120)
        .machine_type("q35")
        .build()
}

fn windows_2019_template() -> VMConfig {
    VMConfigBuilder::new("windows-vm")
        .namespace("default")
        .cpu(4, 1, 1)
        .memory("8Gi")
        .add_blank_disk("rootdisk", "60Gi", 1)
        .add_cdrom("virtio-drivers", "quay.io/containerdisks/virtio-win", 2)
        .add_pod_network("default")
        .label("os", "windows")
        .label("os.version", "2019")
        .features(windows_features())
        .firmware(uefi_firmware())
        .clock(windows_clock())
        .enable_rng()
        .termination_grace_period(120)
        .machine_type("q35")
        .build()
}

fn windows_11_template() -> VMConfig {
    VMConfigBuilder::new("windows-vm")
        .namespace("default")
        .cpu(4, 2, 1)
        .memory("8Gi")
        .add_blank_disk("rootdisk", "80Gi", 1)
        .add_cdrom("virtio-drivers", "quay.io/containerdisks/virtio-win", 2)
        .add_pod_network("default")
        .label("os", "windows")
        .label("os.version", "11")
        .features(windows_features())
        .firmware(uefi_secure_boot_firmware())
        .clock(windows_clock())
        .enable_tpm()
        .enable_rng()
        .termination_grace_period(120)
        .machine_type("q35")
        .build()
}

fn windows_10_template() -> VMConfig {
    VMConfigBuilder::new("windows-vm")
        .namespace("default")
        .cpu(4, 1, 1)
        .memory("8Gi")
        .add_blank_disk("rootdisk", "60Gi", 1)
        .add_cdrom("virtio-drivers", "quay.io/containerdisks/virtio-win", 2)
        .add_pod_network("default")
        .label("os", "windows")
        .label("os.version", "10")
        .features(windows_features())
        .firmware(uefi_firmware())
        .clock(windows_clock())
        .enable_rng()
        .termination_grace_period(120)
        .machine_type("q35")
        .build()
}

// ============================================================================
// FreeBSD Templates
// ============================================================================

fn freebsd_14_template() -> VMConfig {
    VMConfigBuilder::new("freebsd-vm")
        .namespace("default")
        .cpu(2, 1, 1)
        .memory("2Gi")
        .add_blank_disk("rootdisk", "20Gi", 1)
        .add_pod_network("default")
        .label("os", "freebsd")
        .label("os.version", "14")
        .enable_rng()
        .clock(linux_clock())
        .build()
}

fn freebsd_13_template() -> VMConfig {
    VMConfigBuilder::new("freebsd-vm")
        .namespace("default")
        .cpu(2, 1, 1)
        .memory("2Gi")
        .add_blank_disk("rootdisk", "20Gi", 1)
        .add_pod_network("default")
        .label("os", "freebsd")
        .label("os.version", "13")
        .enable_rng()
        .clock(linux_clock())
        .build()
}

// ============================================================================
// Flatcar Linux Template
// ============================================================================

fn flatcar_template() -> VMConfig {
    VMConfigBuilder::new("flatcar-vm")
        .namespace("default")
        .cpu(2, 1, 1)
        .memory("2Gi")
        .add_blank_disk("rootdisk", "20Gi", 1)
        .add_pod_network("default")
        .label("os", "flatcar")
        .label("os.version", "stable")
        .cloud_init(default_cloud_init())
        .enable_rng()
        .clock(linux_clock())
        .build()
}

// ============================================================================
// Talos Linux Template (for Kubernetes)
// ============================================================================

fn talos_template() -> VMConfig {
    VMConfigBuilder::new("talos-vm")
        .namespace("default")
        .cpu(2, 1, 1)
        .memory("4Gi")
        .add_blank_disk("rootdisk", "20Gi", 1)
        .add_pod_network("default")
        .label("os", "talos")
        .label("os.version", "latest")
        .enable_rng()
        .clock(linux_clock())
        .build()
}

// ============================================================================
// Cloud-init configurations
// ============================================================================

// ============================================================================
// Linux containerdisk templates
//
// Every image below must exist on quay.io/containerdisks (checked 2026-09-30).
// Repositories that do NOT exist there: rockylinux, alpine (CDI fails the
// import with "unauthorized"). Re-check a tag before adding one.
// ============================================================================

fn linux_containerdisk_template(
    vm_name: &str,
    os: &str,
    version: &str,
    image: &str,
    cloud_init: String,
) -> VMConfig {
    VMConfigBuilder::new(vm_name)
        .namespace("default")
        .cpu(2, 1, 1)
        .memory("4Gi")
        .add_container_disk("rootdisk", format!("quay.io/containerdisks/{image}"), 1)
        .add_blank_disk("datadisk", "20Gi", 2)
        .add_pod_network("default")
        .label("os", os)
        .label("os.version", version)
        .cloud_init(cloud_init)
        .enable_rng()
        .clock(linux_clock())
        .build()
}

fn ubuntu_2604_template() -> VMConfig {
    linux_containerdisk_template(
        "ubuntu-vm",
        "ubuntu",
        "26.04",
        "ubuntu:26.04",
        default_ubuntu_cloud_init(),
    )
}

fn ubuntu_2404_template() -> VMConfig {
    linux_containerdisk_template(
        "ubuntu-vm",
        "ubuntu",
        "24.04",
        "ubuntu:24.04",
        default_ubuntu_cloud_init(),
    )
}

fn ubuntu_2204_template() -> VMConfig {
    linux_containerdisk_template(
        "ubuntu-vm",
        "ubuntu",
        "22.04",
        "ubuntu:22.04",
        default_ubuntu_cloud_init(),
    )
}

fn fedora_44_template() -> VMConfig {
    linux_containerdisk_template(
        "fedora-vm",
        "fedora",
        "44",
        "fedora:44",
        default_cloud_init(),
    )
}

fn fedora_43_template() -> VMConfig {
    linux_containerdisk_template(
        "fedora-vm",
        "fedora",
        "43",
        "fedora:43",
        default_cloud_init(),
    )
}

fn centos_stream10_template() -> VMConfig {
    linux_containerdisk_template(
        "centos-vm",
        "centos",
        "stream10",
        "centos-stream:10",
        default_cloud_init(),
    )
}

fn centos_stream9_template() -> VMConfig {
    linux_containerdisk_template(
        "centos-vm",
        "centos",
        "stream9",
        "centos-stream:9",
        default_cloud_init(),
    )
}

fn debian_13_template() -> VMConfig {
    linux_containerdisk_template(
        "debian-vm",
        "debian",
        "13",
        "debian:13",
        default_cloud_init(),
    )
}

fn debian_12_template() -> VMConfig {
    linux_containerdisk_template(
        "debian-vm",
        "debian",
        "12",
        "debian:12",
        default_cloud_init(),
    )
}

fn almalinux_10_template() -> VMConfig {
    linux_containerdisk_template(
        "almalinux-vm",
        "almalinux",
        "10",
        "almalinux:10",
        default_cloud_init(),
    )
}

fn almalinux_9_template() -> VMConfig {
    linux_containerdisk_template(
        "almalinux-vm",
        "almalinux",
        "9",
        "almalinux:9",
        default_cloud_init(),
    )
}

fn opensuse_leap_16_template() -> VMConfig {
    linux_containerdisk_template(
        "opensuse-vm",
        "opensuse",
        "leap-16.0",
        "opensuse-leap:16.0",
        default_cloud_init(),
    )
}

fn opensuse_leap_15_template() -> VMConfig {
    linux_containerdisk_template(
        "opensuse-vm",
        "opensuse",
        "leap-15.6",
        "opensuse-leap:15.6",
        default_cloud_init(),
    )
}

fn opensuse_tumbleweed_template() -> VMConfig {
    // Rolling release: the repository publishes a single moving "1.0.0" tag.
    linux_containerdisk_template(
        "opensuse-vm",
        "opensuse",
        "tumbleweed",
        "opensuse-tumbleweed:1.0.0",
        default_cloud_init(),
    )
}

fn default_cloud_init() -> String {
    r#"#cloud-config
user: zorvia
# WARNING: Change this default password immediately
password: zorvia
lock_passwd: false
chpasswd: { expire: True }
ssh_pwauth: False
package_update: true
packages:
  - qemu-guest-agent
runcmd:
  - [ systemctl, enable, qemu-guest-agent ]
  - [ systemctl, start, qemu-guest-agent ]
"#
    .to_string()
}

fn default_ubuntu_cloud_init() -> String {
    r#"#cloud-config
user: ubuntu
# WARNING: Change this default password immediately
password: ubuntu
lock_passwd: false
chpasswd: { expire: True }
ssh_pwauth: False
package_update: true
packages:
  - qemu-guest-agent
runcmd:
  - [ systemctl, enable, qemu-guest-agent ]
  - [ systemctl, start, qemu-guest-agent ]
"#
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_template_manager() {
        let manager = TemplateManager::new();

        assert!(manager.exists("ubuntu"));
        assert!(manager.exists("ubuntu-22.04"));
        assert!(manager.exists("fedora"));
        assert!(manager.exists("almalinux"));
        assert!(!manager.exists("nonexistent"));
        // Aliases point at the newest release.
        assert_eq!(manager.get("ubuntu").unwrap().labels["os.version"], "26.04");
        assert_eq!(manager.get("fedora").unwrap().labels["os.version"], "44");
        assert_eq!(manager.get("debian").unwrap().labels["os.version"], "13");
        assert_eq!(
            manager.get("centos").unwrap().labels["os.version"],
            "stream10"
        );
        assert_eq!(manager.get("almalinux").unwrap().labels["os.version"], "10");
        assert_eq!(
            manager.get("opensuse").unwrap().labels["os.version"],
            "leap-16.0"
        );
        // No public containerdisk exists for these (quay.io/containerdisks
        // has no rockylinux or alpine repository) and the EOL releases were
        // retired, so they must not be offered.
        for gone in [
            "rocky",
            "rocky-8",
            "rocky-9",
            "alpine",
            "alpine-3.19",
            "almalinux-8",
            "ubuntu-20.04",
            "ubuntu-18.04",
            "fedora-41",
            "fedora-40",
            "fedora-39",
            "centos-stream-8",
            "debian-11",
        ] {
            assert!(!manager.exists(gone), "{gone} must not be offered");
        }

        let ubuntu = manager.get("ubuntu").unwrap();
        assert_eq!(ubuntu.cpu.cores, 2);
        assert_eq!(ubuntu.memory.size, "4Gi");
    }

    #[test]
    fn test_list_templates() {
        let manager = TemplateManager::new();
        let templates = manager.list();

        assert!(templates.contains(&"ubuntu".to_string()));
        assert!(templates.contains(&"fedora".to_string()));
        assert!(templates.contains(&"almalinux".to_string()));

        // Should have many templates
        assert!(templates.len() > 30);
    }

    #[test]
    fn test_list_by_family() {
        let manager = TemplateManager::new();
        let families = manager.list_by_family();

        assert!(families.contains_key("Ubuntu"));
        assert!(families.contains_key("Fedora"));
        assert!(families.contains_key("AlmaLinux"));
        assert!(families.contains_key("OpenSUSE"));

        // Ubuntu should have multiple versions
        let ubuntu_templates = families.get("Ubuntu").unwrap();
        assert!(ubuntu_templates.len() >= 4);
    }
}
