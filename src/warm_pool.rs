// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Warm pools of pre-provisioned VMs, ready to claim.
//!
//! "Warm" here means the VM object already exists (from the real template
//! catalog, `crate::templates`) but is left stopped -- claiming it starts
//! it for real. That saves the create-time work (image resolution,
//! cloud-init authoring, object creation), not KubeVirt boot time itself;
//! it is not a hot/running standby pool, which would burn real cluster
//! resources for VMs nobody's using yet. Real, disk-persisted, same shape
//! as `crate::backup::schedule`/`crate::power_schedule`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WarmPoolMemberStatus {
    Provisioning,
    Ready,
    Claimed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WarmPoolMember {
    pub vm_name: String,
    pub status: WarmPoolMemberStatus,
    pub created_at: DateTime<Utc>,
    pub claimed_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WarmPool {
    pub name: String,
    pub template: String,
    pub prefix: String,
    /// Desired number of Ready-or-Provisioning standby members.
    pub size: u32,
    pub members: Vec<WarmPoolMember>,
}

impl WarmPool {
    pub fn new(name: impl Into<String>, template: impl Into<String>, size: u32) -> Self {
        let name = name.into();
        Self {
            prefix: format!("{name}-standby"),
            name,
            template: template.into(),
            size,
            members: Vec::new(),
        }
    }

    pub fn ready_or_provisioning_count(&self) -> usize {
        self.members
            .iter()
            .filter(|m| {
                matches!(
                    m.status,
                    WarmPoolMemberStatus::Ready | WarmPoolMemberStatus::Provisioning
                )
            })
            .count()
    }

    pub fn next_member_name(&self) -> String {
        let mut n = 1u32;
        loop {
            let candidate = format!("{}-{}", self.prefix, n);
            if !self.members.iter().any(|m| m.vm_name == candidate) {
                return candidate;
            }
            n += 1;
        }
    }

    /// The first Ready member, if any -- what `claim` hands out.
    pub fn find_ready(&self) -> Option<&WarmPoolMember> {
        self.members
            .iter()
            .find(|m| m.status == WarmPoolMemberStatus::Ready)
    }
}

#[derive(Serialize, Deserialize)]
pub struct WarmPoolManager {
    pools: Vec<WarmPool>,
}

impl WarmPoolManager {
    fn persistence_path() -> std::path::PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
            .join("zorvia")
            .join("warm_pools.json")
    }

    pub fn load() -> Self {
        let path = Self::persistence_path();
        if let Some(content) = crate::state_docs::read(&path) {
            match serde_json::from_str(&content) {
                Ok(manager) => return manager,
                Err(e) => log::warn!("Failed to parse warm pools: {}", e),
            }
        }
        Self::new()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::persistence_path();
        let content = serde_json::to_string_pretty(self)?;
        crate::state_docs::write(&path, &content)
    }

    pub fn new() -> Self {
        Self { pools: Vec::new() }
    }

    pub fn add_pool(&mut self, pool: WarmPool) {
        self.pools.push(pool);
    }

    pub fn remove_pool(&mut self, name: &str) -> bool {
        let before = self.pools.len();
        self.pools.retain(|p| p.name != name);
        self.pools.len() < before
    }

    pub fn get_pool(&self, name: &str) -> Option<&WarmPool> {
        self.pools.iter().find(|p| p.name == name)
    }

    pub fn get_pool_mut(&mut self, name: &str) -> Option<&mut WarmPool> {
        self.pools.iter_mut().find(|p| p.name == name)
    }

    pub fn all(&self) -> &[WarmPool] {
        &self.pools
    }
}

impl Default for WarmPoolManager {
    fn default() -> Self {
        Self::new()
    }
}
