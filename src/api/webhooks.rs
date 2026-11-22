// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};

/// Check if an IP address is private/internal
fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()           // 127.0.0.0/8
            || v4.is_private()         // 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
            || v4.is_link_local()      // 169.254.0.0/16
            || v4.is_broadcast()       // 255.255.255.255
            || v4.is_unspecified() // 0.0.0.0
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()           // ::1
            || v6.is_unspecified()     // ::
            // fc00::/7 (unique local) and fe80::/10 (link-local)
            || (v6.segments()[0] & 0xfe00) == 0xfc00
            || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

fn webhook_allowlist() -> Vec<String> {
    std::env::var("ZORVIA_WEBHOOK_ALLOWLIST")
        .ok()
        .map(|s| {
            s.split(',')
                .map(|p| p.trim().to_ascii_lowercase())
                .filter(|p| !p.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn host_allowed(host: &str, allowlist: &[String]) -> bool {
    if allowlist.is_empty() {
        return true;
    }
    let lower = host.to_ascii_lowercase();
    allowlist.iter().any(|entry| {
        if let Some(suffix) = entry.strip_prefix('.') {
            lower == suffix || lower.ends_with(&format!(".{}", suffix)) || lower.ends_with(entry)
        } else {
            lower == *entry || lower.ends_with(&format!(".{}", entry))
        }
    })
}

fn extract_webhook_host(url: &str) -> anyhow::Result<(String, u16)> {
    let after_scheme = url
        .strip_prefix("https://")
        .ok_or_else(|| anyhow::anyhow!("Webhook URL must use HTTPS"))?;
    let host_port = after_scheme.split('/').next().unwrap_or("");
    let (host, port) = if host_port.starts_with('[') {
        let end = host_port
            .find(']')
            .ok_or_else(|| anyhow::anyhow!("Webhook URL has invalid IPv6 host"))?;
        let host = host_port[1..end].to_string();
        let port = if host_port[end + 1..].starts_with(':') {
            host_port[end + 2..].parse().unwrap_or(443)
        } else {
            443
        };
        (host, port)
    } else if let Some((h, p)) = host_port.rsplit_once(':') {
        if h.chars().all(|c| c.is_ascii_digit() || c == '.') {
            // host:port for IPv4
            (h.to_string(), p.parse().unwrap_or(443))
        } else if p.chars().all(|c| c.is_ascii_digit()) {
            (h.to_string(), p.parse().unwrap_or(443))
        } else {
            (host_port.to_string(), 443)
        }
    } else {
        (host_port.to_string(), 443)
    };
    if host.is_empty() {
        anyhow::bail!("Webhook URL has no host");
    }
    Ok((host, port))
}

/// Validate that a webhook URL is safe (HTTPS, allowlist, no private addresses
/// after DNS resolution).
fn validate_webhook_url(url: &str) -> anyhow::Result<()> {
    if !url.starts_with("https://") {
        anyhow::bail!("Webhook URL must use HTTPS");
    }

    let (host, port) = extract_webhook_host(url)?;
    let lower = host.to_lowercase();
    if lower == "localhost" || lower.ends_with(".localhost") || lower == "metadata.google.internal"
    {
        anyhow::bail!("Webhook URL must not point to internal addresses");
    }

    let allowlist = webhook_allowlist();
    if !host_allowed(&lower, &allowlist) {
        anyhow::bail!("Webhook host is not on ZORVIA_WEBHOOK_ALLOWLIST");
    }

    // Literal IP
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_private_ip(&ip) {
            anyhow::bail!("Webhook URL must not point to private/internal IP addresses");
        }
        return Ok(());
    }

    // Resolve DNS and reject any private answer (blocks naive rebinding at register time).
    let addrs: Vec<SocketAddr> = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|e| anyhow::anyhow!("Webhook DNS resolution failed: {e}"))?
        .collect();
    if addrs.is_empty() {
        anyhow::bail!("Webhook DNS resolution returned no addresses");
    }
    for addr in &addrs {
        if is_private_ip(&addr.ip()) {
            anyhow::bail!("Webhook URL resolves to a private/internal IP address");
        }
    }

    Ok(())
}

/// Resolve destination and pick a public pinned address for connect.
fn resolve_public_pin(host: &str, port: u16) -> anyhow::Result<SocketAddr> {
    let addrs: Vec<SocketAddr> = (host, port)
        .to_socket_addrs()
        .map_err(|e| anyhow::anyhow!("Webhook DNS resolution failed: {e}"))?
        .collect();
    for addr in addrs {
        if !is_private_ip(&addr.ip()) {
            return Ok(addr);
        }
    }
    anyhow::bail!("Webhook URL resolves only to private/internal addresses")
}

fn validate_destination_url(url: &str) -> anyhow::Result<()> {
    validate_webhook_url(url)
}

/// Webhook event types
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WebhookEvent {
    VMCreated,
    VMDeleted,
    VMStarted,
    VMStopped,
    VMRestarted,
    VMFailed,
    VMHealthChanged,
    SnapshotCreated,
    SnapshotRestored,
    BackupCompleted,
    BackupFailed,
    MigrationStarted,
    MigrationCompleted,
    AlertTriggered,
    AlertResolved,
    Custom(String),
}

impl std::fmt::Display for WebhookEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WebhookEvent::VMCreated => write!(f, "vm.created"),
            WebhookEvent::VMDeleted => write!(f, "vm.deleted"),
            WebhookEvent::VMStarted => write!(f, "vm.started"),
            WebhookEvent::VMStopped => write!(f, "vm.stopped"),
            WebhookEvent::VMRestarted => write!(f, "vm.restarted"),
            WebhookEvent::VMFailed => write!(f, "vm.failed"),
            WebhookEvent::VMHealthChanged => write!(f, "vm.health_changed"),
            WebhookEvent::SnapshotCreated => write!(f, "snapshot.created"),
            WebhookEvent::SnapshotRestored => write!(f, "snapshot.restored"),
            WebhookEvent::BackupCompleted => write!(f, "backup.completed"),
            WebhookEvent::BackupFailed => write!(f, "backup.failed"),
            WebhookEvent::MigrationStarted => write!(f, "migration.started"),
            WebhookEvent::MigrationCompleted => write!(f, "migration.completed"),
            WebhookEvent::AlertTriggered => write!(f, "alert.triggered"),
            WebhookEvent::AlertResolved => write!(f, "alert.resolved"),
            WebhookEvent::Custom(name) => write!(f, "custom.{}", name),
        }
    }
}

/// Webhook configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookConfig {
    pub id: String,
    pub name: String,
    pub url: String,
    pub events: Vec<WebhookEvent>,
    #[serde(skip_serializing)]
    pub secret: Option<String>,
    pub headers: HashMap<String, String>,
    pub enabled: bool,
    pub retry_count: u32,
    pub retry_delay_secs: u64,
    pub timeout_secs: u64,
    pub created_at: DateTime<Utc>,
    pub last_triggered: Option<DateTime<Utc>>,
    pub delivery_count: u64,
    pub failure_count: u64,
}

impl WebhookConfig {
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> anyhow::Result<Self> {
        let name_str = name.into();
        let url_str = url.into();
        validate_webhook_url(&url_str)?;
        Ok(Self {
            id: format!(
                "wh-{}-{}",
                name_str.to_lowercase().replace(' ', "-"),
                Utc::now().timestamp()
            ),
            name: name_str,
            url: url_str,
            events: Vec::new(),
            secret: None,
            headers: HashMap::new(),
            enabled: true,
            retry_count: 3,
            retry_delay_secs: 10,
            timeout_secs: 30,
            created_at: Utc::now(),
            last_triggered: None,
            delivery_count: 0,
            failure_count: 0,
        })
    }

    pub fn add_event(&mut self, event: WebhookEvent) {
        if !self.events.contains(&event) {
            self.events.push(event);
        }
    }

    pub fn remove_event(&mut self, event: &WebhookEvent) -> bool {
        let len_before = self.events.len();
        self.events.retain(|e| e != event);
        self.events.len() < len_before
    }

    pub fn with_secret(mut self, secret: impl Into<String>) -> Self {
        self.secret = Some(secret.into());
        self
    }

    pub fn add_header(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.headers.insert(key.into(), value.into());
    }

    pub fn with_retry(mut self, count: u32, delay_secs: u64) -> Self {
        self.retry_count = count;
        self.retry_delay_secs = delay_secs;
        self
    }

    pub fn disable(&mut self) {
        self.enabled = false;
    }

    pub fn enable(&mut self) {
        self.enabled = true;
    }

    pub fn record_delivery(&mut self, success: bool) {
        self.delivery_count += 1;
        self.last_triggered = Some(Utc::now());
        if !success {
            self.failure_count += 1;
        }
    }

    pub fn subscribes_to(&self, event: &WebhookEvent) -> bool {
        self.events.contains(event)
    }

    pub fn success_rate(&self) -> f64 {
        if self.delivery_count == 0 {
            return 100.0;
        }
        ((self.delivery_count - self.failure_count) as f64 / self.delivery_count as f64) * 100.0
    }

    pub fn event_count(&self) -> usize {
        self.events.len()
    }
}

/// Webhook delivery payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookPayload {
    pub event: String,
    pub timestamp: DateTime<Utc>,
    pub data: HashMap<String, String>,
    pub source: String,
}

impl WebhookPayload {
    pub fn new(event: &WebhookEvent) -> Self {
        Self {
            event: event.to_string(),
            timestamp: Utc::now(),
            data: HashMap::new(),
            source: "zorvia".to_string(),
        }
    }

    pub fn add_data(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.data.insert(key.into(), value.into());
    }

    pub fn data_count(&self) -> usize {
        self.data.len()
    }
}

/// Webhook manager
#[derive(Serialize, Deserialize)]
pub struct WebhookManager {
    webhooks: HashMap<String, WebhookConfig>,
}

impl WebhookManager {
    pub fn new() -> Self {
        Self {
            webhooks: HashMap::new(),
        }
    }

    fn persistence_path() -> std::path::PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
            .join("zorvia")
            .join("webhooks.json")
    }

    pub fn load() -> Self {
        let path = Self::persistence_path();
        if let Some(content) = crate::state_docs::read(&path) {
            match serde_json::from_str(&content) {
                Ok(manager) => return manager,
                Err(e) => log::warn!("Failed to parse webhooks: {}", e),
            }
        }
        Self::new()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::persistence_path();
        let content = serde_json::to_string_pretty(self)?;
        crate::state_docs::write(&path, &content)
    }

    /// Delivers `payload` to every enabled webhook subscribed to `event`,
    /// via a real HTTPS POST (`reqwest`), honoring each webhook's own
    /// retry count/delay. The configured `secret`, if any, goes in an
    /// `X-Zorvia-Webhook-Secret` header -- a shared-secret check, not an
    /// HMAC body signature (this crate doesn't otherwise depend on `hmac`,
    /// and adding it for this alone wasn't worth a new dependency).
    /// Delivery stats are recorded and persisted either way.
    #[cfg(feature = "web")]
    pub async fn dispatch(&mut self, event: &WebhookEvent, payload: &WebhookPayload) {
        let ids: Vec<String> = self
            .webhooks_for_event(event)
            .into_iter()
            .map(|w| w.id.clone())
            .collect();
        if ids.is_empty() {
            return;
        }
        for id in ids {
            let Some(config) = self.webhooks.get(&id).cloned() else {
                continue;
            };
            let success = deliver_with_retry(&config, payload).await;
            if let Some(w) = self.webhooks.get_mut(&id) {
                w.record_delivery(success);
            }
        }
        if let Err(e) = self.save() {
            log::warn!("Failed to persist webhook delivery stats: {}", e);
        }
    }

    pub fn register(&mut self, webhook: WebhookConfig) -> String {
        let id = webhook.id.clone();
        self.webhooks.insert(id.clone(), webhook);
        id
    }

    pub fn unregister(&mut self, id: &str) -> bool {
        self.webhooks.remove(id).is_some()
    }

    pub fn get(&self, id: &str) -> Option<&WebhookConfig> {
        self.webhooks.get(id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut WebhookConfig> {
        self.webhooks.get_mut(id)
    }

    pub fn list(&self) -> Vec<&WebhookConfig> {
        self.webhooks.values().collect()
    }

    pub fn active_webhooks(&self) -> Vec<&WebhookConfig> {
        self.webhooks.values().filter(|w| w.enabled).collect()
    }

    pub fn webhooks_for_event(&self, event: &WebhookEvent) -> Vec<&WebhookConfig> {
        self.webhooks
            .values()
            .filter(|w| w.enabled && w.subscribes_to(event))
            .collect()
    }

    pub fn webhook_count(&self) -> usize {
        self.webhooks.len()
    }
}

impl Default for WebhookManager {
    fn default() -> Self {
        Self::new()
    }
}

/// A single delivery attempt with DNS pin + redirect revalidation.
#[cfg(feature = "web")]
pub async fn deliver_once(config: &WebhookConfig, payload: &WebhookPayload) -> anyhow::Result<()> {
    // Re-validate at delivery time (DNS may have changed / rebinding).
    validate_destination_url(&config.url)?;
    let (host, port) = extract_webhook_host(&config.url)?;
    let pinned = resolve_public_pin(&host, port)?;

    let redirect_policy = reqwest::redirect::Policy::custom(|attempt| {
        let next = attempt.url().as_str();
        if validate_destination_url(next).is_err() {
            attempt.error(anyhow::anyhow!(
                "webhook redirect to disallowed destination"
            ))
        } else {
            attempt.follow()
        }
    });

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(config.timeout_secs))
        .redirect(redirect_policy)
        .resolve(&host, pinned)
        .build()?;
    let mut req = client.post(&config.url).json(payload);
    if let Some(secret) = &config.secret {
        req = req.header("X-Zorvia-Webhook-Secret", secret);
    }
    for (k, v) in &config.headers {
        req = req.header(k, v);
    }
    let resp = req.send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("webhook endpoint returned {}", resp.status());
    }
    Ok(())
}

/// Delivers with the webhook's own configured retry count/delay. Returns
/// whether any attempt succeeded.
#[cfg(feature = "web")]
async fn deliver_with_retry(config: &WebhookConfig, payload: &WebhookPayload) -> bool {
    let attempts = config.retry_count.max(1);
    for attempt in 0..attempts {
        match deliver_once(config, payload).await {
            Ok(()) => return true,
            Err(e) => {
                log::warn!(
                    "Webhook '{}' delivery attempt {}/{} failed: {e}",
                    config.name,
                    attempt + 1,
                    attempts
                );
                if attempt + 1 < attempts {
                    tokio::time::sleep(std::time::Duration::from_secs(config.retry_delay_secs))
                        .await;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_webhook_event_display() {
        assert_eq!(WebhookEvent::VMCreated.to_string(), "vm.created");
        assert_eq!(WebhookEvent::VMDeleted.to_string(), "vm.deleted");
        assert_eq!(WebhookEvent::VMStarted.to_string(), "vm.started");
        assert_eq!(
            WebhookEvent::BackupCompleted.to_string(),
            "backup.completed"
        );
        assert_eq!(WebhookEvent::AlertTriggered.to_string(), "alert.triggered");
        assert_eq!(
            WebhookEvent::Custom("deploy".to_string()).to_string(),
            "custom.deploy"
        );
    }

    #[test]
    fn test_webhook_config_new() {
        let wh = WebhookConfig::new("test", "https://example.com/webhook").unwrap();
        assert_eq!(wh.name, "test");
        assert_eq!(wh.url, "https://example.com/webhook");
        assert!(wh.enabled);
        assert_eq!(wh.retry_count, 3);
        assert_eq!(wh.delivery_count, 0);
    }

    #[test]
    fn test_webhook_rejects_http() {
        assert!(WebhookConfig::new("test", "http://example.com").is_err());
    }

    #[test]
    fn test_webhook_rejects_internal_urls() {
        assert!(WebhookConfig::new("test", "https://127.0.0.1/hook").is_err());
        assert!(WebhookConfig::new("test", "https://localhost/hook").is_err());
        assert!(WebhookConfig::new("test", "https://169.254.169.254/latest").is_err());
        assert!(WebhookConfig::new("test", "https://192.168.1.1/hook").is_err());
        assert!(WebhookConfig::new("test", "https://10.0.0.1/hook").is_err());
        assert!(WebhookConfig::new("test", "https://[::1]/hook").is_err());
    }

    #[test]
    fn test_webhook_add_event() {
        let mut wh = WebhookConfig::new("test", "https://example.com").unwrap();
        wh.add_event(WebhookEvent::VMCreated);
        wh.add_event(WebhookEvent::VMDeleted);
        wh.add_event(WebhookEvent::VMCreated); // Duplicate

        assert_eq!(wh.event_count(), 2);
        assert!(wh.subscribes_to(&WebhookEvent::VMCreated));
    }

    #[test]
    fn test_webhook_remove_event() {
        let mut wh = WebhookConfig::new("test", "https://example.com").unwrap();
        wh.add_event(WebhookEvent::VMCreated);

        assert!(wh.remove_event(&WebhookEvent::VMCreated));
        assert!(!wh.remove_event(&WebhookEvent::VMCreated));
        assert_eq!(wh.event_count(), 0);
    }

    #[test]
    fn test_webhook_with_secret() {
        let wh = WebhookConfig::new("test", "https://example.com")
            .unwrap()
            .with_secret("my-secret");
        assert_eq!(wh.secret, Some("my-secret".to_string()));
    }

    #[test]
    fn test_webhook_add_header() {
        let mut wh = WebhookConfig::new("test", "https://example.com").unwrap();
        wh.add_header("X-Custom", "value");
        assert_eq!(wh.headers.get("X-Custom"), Some(&"value".to_string()));
    }

    #[test]
    fn test_webhook_with_retry() {
        let wh = WebhookConfig::new("test", "https://example.com")
            .unwrap()
            .with_retry(5, 30);
        assert_eq!(wh.retry_count, 5);
        assert_eq!(wh.retry_delay_secs, 30);
    }

    #[test]
    fn test_webhook_enable_disable() {
        let mut wh = WebhookConfig::new("test", "https://example.com").unwrap();
        assert!(wh.enabled);

        wh.disable();
        assert!(!wh.enabled);

        wh.enable();
        assert!(wh.enabled);
    }

    #[test]
    fn test_webhook_record_delivery() {
        let mut wh = WebhookConfig::new("test", "https://example.com").unwrap();

        wh.record_delivery(true);
        wh.record_delivery(true);
        wh.record_delivery(false);

        assert_eq!(wh.delivery_count, 3);
        assert_eq!(wh.failure_count, 1);
        assert!(wh.last_triggered.is_some());
    }

    #[test]
    fn test_webhook_success_rate() {
        let mut wh = WebhookConfig::new("test", "https://example.com").unwrap();
        assert_eq!(wh.success_rate(), 100.0);

        wh.record_delivery(true);
        wh.record_delivery(false);

        assert_eq!(wh.success_rate(), 50.0);
    }

    #[test]
    fn test_webhook_payload_new() {
        let mut payload = WebhookPayload::new(&WebhookEvent::VMCreated);
        payload.add_data("vm_name", "test-vm");
        payload.add_data("namespace", "default");

        assert_eq!(payload.event, "vm.created");
        assert_eq!(payload.data_count(), 2);
        assert_eq!(payload.source, "zorvia");
    }

    #[test]
    fn test_webhook_manager_register() {
        let mut manager = WebhookManager::new();
        let wh = WebhookConfig::new("test", "https://example.com").unwrap();
        let id = manager.register(wh);

        assert_eq!(manager.webhook_count(), 1);
        assert!(manager.get(&id).is_some());
    }

    #[test]
    fn test_webhook_manager_unregister() {
        let mut manager = WebhookManager::new();
        let id = manager.register(WebhookConfig::new("test", "https://example.com").unwrap());

        assert!(manager.unregister(&id));
        assert!(!manager.unregister(&id));
    }

    #[test]
    fn test_webhook_manager_active_webhooks() {
        let mut manager = WebhookManager::new();

        let mut disabled = WebhookConfig::new("disabled", "https://example.com").unwrap();
        disabled.disable();

        manager.register(WebhookConfig::new("active", "https://example.com").unwrap());
        manager.register(disabled);

        assert_eq!(manager.active_webhooks().len(), 1);
    }

    #[test]
    fn test_webhook_manager_webhooks_for_event() {
        let mut manager = WebhookManager::new();

        let mut wh1 = WebhookConfig::new("vm-events", "https://example.com/vm").unwrap();
        wh1.add_event(WebhookEvent::VMCreated);
        wh1.add_event(WebhookEvent::VMDeleted);

        let mut wh2 = WebhookConfig::new("backup-events", "https://example.com/backup").unwrap();
        wh2.add_event(WebhookEvent::BackupCompleted);

        manager.register(wh1);
        manager.register(wh2);

        let vm_hooks = manager.webhooks_for_event(&WebhookEvent::VMCreated);
        assert_eq!(vm_hooks.len(), 1);

        let backup_hooks = manager.webhooks_for_event(&WebhookEvent::BackupCompleted);
        assert_eq!(backup_hooks.len(), 1);

        let no_hooks = manager.webhooks_for_event(&WebhookEvent::AlertTriggered);
        assert_eq!(no_hooks.len(), 0);
    }
}
