// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Installing an in-guest agent at VM creation, through cloud-init.
//!
//! Two agents are supported: the **Zyvor guest agent** (GuestKit's `guestkitd`,
//! a QEMU-guest-agent-compatible replacement that also does fsfreeze/thaw, exec
//! and the GuestKit evidence methods) and stock `qemu-guest-agent`. Either makes
//! KubeVirt report `agentConnected`, which gives application-consistent online
//! snapshots (the guest filesystem is frozen around the snapshot) and lets drills
//! and readiness checks see inside the guest.
//!
//! The Zyvor agent is fetched by the guest itself from a pinned release URL and
//! its SHA-256 is checked before installation (`sha256sum -c`), so a changed or
//! tampered download is not installed. Mirror it internally (air-gapped labs)
//! with `ZORVIA_GUEST_AGENT_URL` + `ZORVIA_GUEST_AGENT_SHA256`. The package is a
//! `.deb`: Debian-family guests only. Everything here is pure and unit-tested.

use anyhow::{anyhow, bail, Result};
use serde_yaml::{Mapping, Value};

/// systemd drop-in for packages older than GuestKit 1.2.5. Their unit runs the agent as the
/// unprivileged `zyvor-agent` user with an empty capability bounding set and their QGA freeze
/// handlers ran `fsfreeze` themselves, so it failed with "Operation not permitted" and every
/// online snapshot of a VM with PVC-backed disks failed at the freeze (measured on a live
/// guest). Freezing needs only CAP_SYS_ADMIN, granted as an ambient capability and nothing
/// else. GuestKit 1.2.5 freezes through its privileged helper instead, so the default package
/// does not need it; it is only written when `ZORVIA_GUEST_AGENT_URL` points somewhere else
/// (a mirror or an older build we cannot vouch for).
pub const FREEZE_DROPIN_PATH: &str =
    "/etc/systemd/system/guestkit-agent.service.d/10-zorvia-freeze.conf";
pub const FREEZE_DROPIN: &str =
    "[Service]\nCapabilityBoundingSet=CAP_SYS_ADMIN\nAmbientCapabilities=CAP_SYS_ADMIN\n";

/// Pinned GuestKit release shipped by default.
pub const DEFAULT_VERSION: &str = "1.2.5";
pub const DEFAULT_URL: &str =
    "https://github.com/zyvorai/guestkit/releases/download/v1.2.5/zyvor-vm-tools_1.2.5_amd64.deb";
pub const DEFAULT_SHA256: &str = "9d2c211a7b1b372b14f602f0812d9596942776090e0fcfcd5b3a486803eb101c";

/// Which agent to install.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentKind {
    None,
    Zyvor,
    Qemu,
}

impl AgentKind {
    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "none" | "off" | "false" => Ok(Self::None),
            "zyvor" | "guestkit" => Ok(Self::Zyvor),
            "qemu" | "qemu-guest-agent" => Ok(Self::Qemu),
            other => bail!("unknown guest_agent '{other}' (use zyvor, qemu or none)"),
        }
    }

    /// The server-wide default (`ZORVIA_GUEST_AGENT`), `none` when unset or invalid.
    pub fn from_env_default() -> Self {
        std::env::var("ZORVIA_GUEST_AGENT")
            .ok()
            .and_then(|v| Self::parse(&v).ok())
            .unwrap_or(Self::None)
    }
}

/// Where the guest downloads the Zyvor agent from, and the checksum it must match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentPackage {
    pub url: String,
    pub sha256: String,
}

impl AgentPackage {
    pub fn from_env() -> Result<Self> {
        let url = std::env::var("ZORVIA_GUEST_AGENT_URL").unwrap_or_else(|_| DEFAULT_URL.into());
        let sha256 =
            std::env::var("ZORVIA_GUEST_AGENT_SHA256").unwrap_or_else(|_| DEFAULT_SHA256.into());
        Self::new(url, sha256)
    }

    /// The URL and checksum end up inside a shell command in the guest, so both
    /// are held to a strict character set (and https only).
    pub fn new(url: String, sha256: String) -> Result<Self> {
        let url_ok = url.starts_with("https://")
            && url.len() <= 512
            && url
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-._~:/?&=%+@".contains(&b));
        if !url_ok {
            bail!("guest agent URL must be https:// with only URL-safe characters");
        }
        let sha = sha256.trim().to_ascii_lowercase();
        if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("guest agent SHA-256 must be 64 hex characters");
        }
        Ok(Self { url, sha256: sha })
    }

    /// True for the pinned release, whose agent freezes through its privileged helper.
    pub fn is_default(&self) -> bool {
        self.url == DEFAULT_URL && self.sha256 == DEFAULT_SHA256
    }
}

/// udev rule that hands the hypervisor channel to the agent's unprivileged user.
/// GuestKit 1.2.5 ships the same rule; cloud-init still writes it (identical content,
/// harmless) so older or mirrored packages work too.
const UDEV_RULE: &str = "# Let the unprivileged zyvor-agent user open the hypervisor channel.\n\
SUBSYSTEM==\"virtio-ports\", ATTR{name}==\"org.qemu.guest_agent.0\", GROUP=\"zyvor-agent\", MODE=\"0660\"\n\
SUBSYSTEM==\"virtio-ports\", ATTR{name}==\"com.zyvor.guestkit.0\", GROUP=\"zyvor-agent\", MODE=\"0660\"\n";

fn cmd(argv: &[&str]) -> Value {
    Value::Sequence(
        argv.iter()
            .map(|a| Value::String((*a).to_string()))
            .collect(),
    )
}

fn append(map: &mut Mapping, key: &str, items: Vec<Value>) -> Result<()> {
    let k = Value::String(key.to_string());
    match map.get_mut(&k) {
        None | Some(Value::Null) => {
            map.insert(k, Value::Sequence(items));
        }
        Some(Value::Sequence(existing)) => existing.extend(items),
        Some(_) => bail!("cloud-config `{key}` must be a list to add the guest agent"),
    }
    Ok(())
}

/// Merge the agent installation into cloud-config `user_data` and return the new
/// document. Anything that is not a `#cloud-config` mapping is refused rather than
/// silently dropped.
pub fn merge_cloud_init(user_data: &str, kind: AgentKind, pkg: &AgentPackage) -> Result<String> {
    if kind == AgentKind::None {
        return Ok(user_data.to_string());
    }
    let body = user_data.strip_prefix("#cloud-config").ok_or_else(|| {
        anyhow!("guest_agent needs cloud-config user data (starting with #cloud-config)")
    })?;
    let mut doc: Value = if body.trim().is_empty() {
        Value::Mapping(Mapping::new())
    } else {
        serde_yaml::from_str(body)
            .map_err(|e| anyhow!("user data is not valid cloud-config: {e}"))?
    };
    let map = doc
        .as_mapping_mut()
        .ok_or_else(|| anyhow!("cloud-config must be a mapping to add the guest agent"))?;

    match kind {
        AgentKind::Qemu => {
            append(
                map,
                "packages",
                vec![Value::String("qemu-guest-agent".into())],
            )?;
            append(
                map,
                "runcmd",
                vec![cmd(&["systemctl", "enable", "--now", "qemu-guest-agent"])],
            )?;
        }
        AgentKind::Zyvor => {
            let mut rule = Mapping::new();
            rule.insert(
                "path".into(),
                Value::String("/etc/udev/rules.d/60-zyvor-guest-agent.rules".into()),
            );
            rule.insert("content".into(), Value::String(UDEV_RULE.into()));
            let mut files = vec![Value::Mapping(rule)];
            if !pkg.is_default() {
                let mut dropin = Mapping::new();
                dropin.insert("path".into(), Value::String(FREEZE_DROPIN_PATH.into()));
                dropin.insert("content".into(), Value::String(FREEZE_DROPIN.into()));
                files.push(Value::Mapping(dropin));
            }
            append(map, "write_files", files)?;
            let install = format!(
                "command -v dpkg >/dev/null 2>&1 || {{ echo 'zyvor guest agent: needs a Debian-family guest' >&2; exit 0; }}; \
                 set -e; curl -fsSL -o /tmp/zyvor-vm-tools.deb '{url}'; \
                 echo '{sha}  /tmp/zyvor-vm-tools.deb' | sha256sum -c -; \
                 dpkg -i /tmp/zyvor-vm-tools.deb; rm -f /tmp/zyvor-vm-tools.deb",
                url = pkg.url,
                sha = pkg.sha256
            );
            append(
                map,
                "runcmd",
                vec![
                    cmd(&["sh", "-c", &install]),
                    cmd(&["udevadm", "control", "--reload"]),
                    cmd(&[
                        "udevadm",
                        "trigger",
                        "--subsystem-match=virtio-ports",
                        "--action=change",
                    ]),
                    cmd(&["udevadm", "settle", "--timeout=10"]),
                    cmd(&["systemctl", "daemon-reload"]),
                    cmd(&["systemctl", "enable", "--now", "guestkit-agent.service"]),
                    cmd(&["systemctl", "restart", "guestkit-agent.service"]),
                ],
            )?;
        }
        AgentKind::None => unreachable!(),
    }
    Ok(format!("#cloud-config\n{}", serde_yaml::to_string(&doc)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg() -> AgentPackage {
        AgentPackage::new(DEFAULT_URL.into(), DEFAULT_SHA256.into()).unwrap()
    }

    #[test]
    fn kinds_parse_and_unknown_is_rejected() {
        assert_eq!(AgentKind::parse("zyvor").unwrap(), AgentKind::Zyvor);
        assert_eq!(AgentKind::parse("GuestKit").unwrap(), AgentKind::Zyvor);
        assert_eq!(AgentKind::parse("qemu").unwrap(), AgentKind::Qemu);
        assert_eq!(AgentKind::parse("").unwrap(), AgentKind::None);
        assert!(AgentKind::parse("vmware-tools").is_err());
    }

    #[test]
    fn package_rejects_unsafe_urls_and_bad_checksums() {
        assert!(AgentPackage::new(DEFAULT_URL.into(), DEFAULT_SHA256.into()).is_ok());
        for bad in [
            "http://insecure/x.deb",
            "https://a/b.deb'; rm -rf / #",
            "https://a/b c.deb",
            "file:///etc/passwd",
            "https://a/$(id).deb",
        ] {
            assert!(
                AgentPackage::new(bad.into(), DEFAULT_SHA256.into()).is_err(),
                "{bad}"
            );
        }
        assert!(AgentPackage::new(DEFAULT_URL.into(), "abc".into()).is_err());
        assert!(AgentPackage::new(DEFAULT_URL.into(), "z".repeat(64)).is_err());
    }

    #[test]
    fn none_leaves_user_data_untouched() {
        let ud = "#!/bin/sh\necho hi\n";
        assert_eq!(merge_cloud_init(ud, AgentKind::None, &pkg()).unwrap(), ud);
    }

    #[test]
    fn zyvor_is_installed_checksum_verified_with_the_udev_rule_and_keeps_user_config() {
        let ud = "#cloud-config\nhostname: web\nruncmd:\n  - [echo, first]\nwrite_files:\n  - path: /etc/x\n    content: y\n";
        let out = merge_cloud_init(ud, AgentKind::Zyvor, &pkg()).unwrap();
        assert!(out.starts_with("#cloud-config\n"));
        let v: Value = serde_yaml::from_str(&out["#cloud-config".len()..]).unwrap();
        assert_eq!(v["hostname"], Value::String("web".into()));
        let run = v["runcmd"].as_sequence().unwrap();
        assert_eq!(run[0], cmd(&["echo", "first"]), "user commands come first");
        let install = run[1][2].as_str().unwrap();
        assert!(install.contains(DEFAULT_URL) && install.contains("sha256sum -c -"));
        assert!(install.contains(DEFAULT_SHA256));
        assert!(run
            .iter()
            .any(|c| c == &cmd(&["systemctl", "restart", "guestkit-agent.service"])));
        let files = v["write_files"].as_sequence().unwrap();
        assert_eq!(
            files.len(),
            2,
            "user file kept and udev rule added; the pinned 1.2.5 package needs no freeze drop-in"
        );
        let rule = files[1]["content"].as_str().unwrap();
        assert!(rule.contains("org.qemu.guest_agent.0") && rule.contains("zyvor-agent"));
    }

    #[test]
    fn a_mirrored_or_older_package_gets_the_freeze_drop_in() {
        let other = AgentPackage::new(
            "https://mirror.example.com/zyvor-vm-tools_1.2.4_amd64.deb".into(),
            "a".repeat(64),
        )
        .unwrap();
        assert!(!other.is_default() && pkg().is_default());
        let out = merge_cloud_init("#cloud-config\n", AgentKind::Zyvor, &other).unwrap();
        let v: Value = serde_yaml::from_str(&out["#cloud-config".len()..]).unwrap();
        let files = v["write_files"].as_sequence().unwrap();
        assert_eq!(files.len(), 2);
        let dropin = files[1]["content"].as_str().unwrap();
        assert!(files[1]["path"]
            .as_str()
            .unwrap()
            .ends_with("10-zorvia-freeze.conf"));
        assert!(dropin.contains("AmbientCapabilities=CAP_SYS_ADMIN"));
        assert!(
            !dropin.contains("CAP_NET") && !dropin.contains("User="),
            "only the freeze capability"
        );
    }

    #[test]
    fn qemu_adds_the_package_and_enables_it() {
        let out = merge_cloud_init("#cloud-config\n", AgentKind::Qemu, &pkg()).unwrap();
        let v: Value = serde_yaml::from_str(&out["#cloud-config".len()..]).unwrap();
        assert_eq!(v["packages"][0], Value::String("qemu-guest-agent".into()));
        assert!(v["runcmd"].as_sequence().unwrap().contains(&cmd(&[
            "systemctl",
            "enable",
            "--now",
            "qemu-guest-agent"
        ])));
    }

    #[test]
    fn non_cloud_config_user_data_is_refused_not_dropped() {
        assert!(merge_cloud_init("#!/bin/sh\necho hi\n", AgentKind::Zyvor, &pkg()).is_err());
        assert!(merge_cloud_init("#cloud-config\n- a\n- b\n", AgentKind::Zyvor, &pkg()).is_err());
        assert!(merge_cloud_init(
            "#cloud-config\nruncmd: not-a-list\n",
            AgentKind::Zyvor,
            &pkg()
        )
        .is_err());
    }
}
