// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! VM power scheduling -- start/stop/restart a VM on a recurring schedule.
//! Real, disk-persisted, same shape as `crate::backup::schedule` (whose
//! `ScheduleType` this reuses directly, including its already-tested
//! next-run math), but triggers VM power actions instead of backups.

use crate::backup::schedule::ScheduleType;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PowerAction {
    Start,
    Stop,
    Restart,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PowerSchedule {
    pub name: String,
    pub vm_name: String,
    pub action: PowerAction,
    pub schedule_type: ScheduleType,
    pub enabled: bool,
    pub next_run: Option<DateTime<Utc>>,
    pub last_run: Option<DateTime<Utc>>,
}

impl PowerSchedule {
    pub fn new(
        name: impl Into<String>,
        vm_name: impl Into<String>,
        action: PowerAction,
        schedule_type: ScheduleType,
    ) -> Self {
        Self {
            name: name.into(),
            vm_name: vm_name.into(),
            action,
            schedule_type,
            enabled: true,
            next_run: None,
            last_run: None,
        }
    }

    pub fn calculate_next_run(&self, from: DateTime<Utc>) -> Option<DateTime<Utc>> {
        if !self.enabled {
            return None;
        }
        self.schedule_type.next_run(from)
    }
}

#[derive(Serialize, Deserialize)]
pub struct PowerScheduleManager {
    schedules: Vec<PowerSchedule>,
}

impl PowerScheduleManager {
    fn persistence_path() -> std::path::PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
            .join("zorvia")
            .join("power_schedules.json")
    }

    pub fn load() -> Self {
        let path = Self::persistence_path();
        if let Some(content) = crate::state_docs::read(&path) {
            match serde_json::from_str(&content) {
                Ok(manager) => return manager,
                Err(e) => log::warn!("Failed to parse power schedules: {}", e),
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
        Self {
            schedules: Vec::new(),
        }
    }

    pub fn add_schedule(&mut self, schedule: PowerSchedule) {
        self.schedules.push(schedule);
        if let Err(e) = self.save() {
            log::warn!("Failed to persist power schedules: {}", e);
        }
    }

    pub fn remove_schedule(&mut self, name: &str) -> bool {
        if let Some(pos) = self.schedules.iter().position(|s| s.name == name) {
            self.schedules.remove(pos);
            if let Err(e) = self.save() {
                log::warn!("Failed to persist power schedules: {}", e);
            }
            true
        } else {
            false
        }
    }

    pub fn get_schedule(&self, name: &str) -> Option<&PowerSchedule> {
        self.schedules.iter().find(|s| s.name == name)
    }

    pub fn all(&self) -> &[PowerSchedule] {
        &self.schedules
    }

    pub fn get_due_schedules(&self, now: DateTime<Utc>) -> Vec<&PowerSchedule> {
        self.schedules
            .iter()
            .filter(|s| s.enabled && s.next_run.map(|next| next <= now).unwrap_or(false))
            .collect()
    }
}

impl Default for PowerScheduleManager {
    fn default() -> Self {
        Self::new()
    }
}
