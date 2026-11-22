// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Read-only, explicitly enrolled Kubernetes fleet. Never accepts credentials from HTTP.
use chrono::{DateTime, Utc};
use futures_util::{stream, StreamExt};
use k8s_openapi::api::core::v1::Node;
use kube::{
    api::{ApiResource, DynamicObject, ListParams},
    config::{KubeConfigOptions, Kubeconfig},
    Api, Client, Config, Resource,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::Path,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

const MAX_CLUSTERS: usize = 16;
const MAX_OBJECTS: usize = 5000;
const CACHE_TTL: Duration = Duration::from_secs(15);
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Enrollment {
    pub name: String,
    pub kubeconfig: String,
    pub context: String,
    pub namespaces: Vec<String>,
    #[serde(default)]
    pub environment: String,
    #[serde(default)]
    pub region: String,
}

#[derive(Clone)]
enum Connection {
    Local(Client),
    Remote(Enrollment),
}
#[derive(Clone)]
struct Target {
    name: String,
    namespaces: Vec<String>,
    environment: String,
    region: String,
    connection: Connection,
}

pub struct FleetService {
    targets: Vec<Target>,
    // Holding this through refresh coalesces concurrent requests; bounds remote fan-out.
    cache: Mutex<Option<(Instant, FleetSnapshot)>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct FleetSnapshot {
    pub observed_at: DateTime<Utc>,
    pub clusters: Vec<ClusterSnapshot>,
    pub totals: FleetTotals,
}
#[derive(Clone, Debug, Serialize)]
pub struct FleetTotals {
    pub configured_clusters: usize,
    pub complete_clusters: usize,
    pub partial: bool,
    pub observed_vms: usize,
    pub observed_nodes: usize,
}
#[derive(Clone, Debug, Serialize)]
pub struct ClusterSnapshot {
    pub name: String,
    pub namespaces: Vec<String>,
    pub environment: String,
    pub region: String,
    pub observed_at: DateTime<Utc>,
    pub health: String,
    // None is unknown, never zero. A failed component discards its partial pages.
    pub node_count: Option<usize>,
    pub ready_nodes: Option<usize>,
    pub vm_count: Option<usize>,
    pub ready_vms: Option<usize>,
    pub vms: Option<Vec<VmInventory>>,
    pub issues: Vec<String>,
}
#[derive(Clone, Debug, Serialize)]
pub struct VmInventory {
    pub namespace: String,
    pub name: String,
    pub status: String,
    pub ready: Option<bool>,
}

fn dns_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value.as_bytes()[value.len() - 1].is_ascii_alphanumeric()
}
fn parse_enrollments(text: &str) -> anyhow::Result<Vec<Enrollment>> {
    let entries: Vec<Enrollment> = serde_json::from_str(text).map_err(|_| {
        anyhow::anyhow!("Fleet registry must be a JSON array with valid enrollment fields")
    })?;
    anyhow::ensure!(
        entries.len() <= MAX_CLUSTERS,
        "At most 16 remote fleet clusters are allowed"
    );
    let mut names = HashSet::from(["local".to_string()]);
    for entry in &entries {
        anyhow::ensure!(
            dns_label(&entry.name) && names.insert(entry.name.clone()),
            "Fleet cluster names must be unique DNS labels; local is reserved"
        );
        anyhow::ensure!(
            Path::new(&entry.kubeconfig).is_absolute(),
            "Fleet kubeconfig paths must be absolute"
        );
        anyhow::ensure!(
            !entry.context.trim().is_empty() && entry.context.len() <= 253,
            "An explicit fleet kubeconfig context is required"
        );
        anyhow::ensure!(
            !entry.namespaces.is_empty() && entry.namespaces.len() <= 32,
            "Each fleet cluster requires 1–32 explicit namespaces"
        );
        let mut namespaces = HashSet::new();
        anyhow::ensure!(
            entry
                .namespaces
                .iter()
                .all(|n| dns_label(n) && namespaces.insert(n)),
            "Fleet namespaces must be unique DNS labels"
        );
        anyhow::ensure!(
            [&entry.environment, &entry.region]
                .iter()
                .all(|s| s.len() <= 128 && !s.chars().any(char::is_control)),
            "Fleet metadata must be at most 128 characters without control characters"
        );
    }
    Ok(entries)
}

// Bounded read, including when a mounted file changes between metadata and read.
async fn read_config(path: &str) -> anyhow::Result<String> {
    use tokio::io::AsyncReadExt;
    let file = tokio::fs::File::open(path).await?;
    let mut bytes = Vec::new();
    file.take(MAX_CONFIG_BYTES + 1)
        .read_to_end(&mut bytes)
        .await?;
    anyhow::ensure!(
        bytes.len() as u64 <= MAX_CONFIG_BYTES,
        "Fleet config exceeds 1 MiB"
    );
    Ok(String::from_utf8(bytes)?)
}

// Only embedded, TLS-verified credentials. No subprocesses, auth providers or
// arbitrary credential file reads during a request. This applies to all contexts.
fn validate_kubeconfig(kubeconfig: &Kubeconfig) -> anyhow::Result<()> {
    for user in &kubeconfig.auth_infos {
        if let Some(auth) = &user.auth_info {
            anyhow::ensure!(
                auth.exec.is_none() && auth.auth_provider.is_none(),
                "Fleet credential plugins are unsupported"
            );
            anyhow::ensure!(
                auth.token_file.is_none()
                    && auth.client_certificate.is_none()
                    && auth.client_key.is_none(),
                "Fleet credentials must be embedded"
            );
        }
    }
    for cluster in &kubeconfig.clusters {
        if let Some(cluster) = &cluster.cluster {
            anyhow::ensure!(
                cluster.insecure_skip_tls_verify != Some(true)
                    && cluster.certificate_authority.is_none()
                    && cluster.proxy_url.is_none(),
                "Fleet requires verified TLS with embedded CA and no proxy override"
            );
            let url = reqwest::Url::parse(cluster.server.as_deref().unwrap_or_default())?;
            anyhow::ensure!(
                url.scheme() == "https"
                    && url.host_str().is_some()
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.query().is_none()
                    && url.fragment().is_none(),
                "Fleet server must be an HTTPS URL without credentials, query or fragment"
            );
        }
    }
    Ok(())
}
async fn remote_client(entry: &Enrollment) -> anyhow::Result<Client> {
    let contents = read_config(&entry.kubeconfig).await?;
    let kubeconfig = Kubeconfig::from_yaml(&contents)?;
    validate_kubeconfig(&kubeconfig)?;
    let mut config = Config::from_custom_kubeconfig(
        kubeconfig,
        &KubeConfigOptions {
            context: Some(entry.context.clone()),
            ..Default::default()
        },
    )
    .await?;
    config.connect_timeout = Some(Duration::from_secs(3));
    config.read_timeout = Some(PROBE_TIMEOUT);
    Ok(Client::try_from(config)?)
}

impl FleetService {
    pub async fn from_env(local: Client, namespace: String) -> anyhow::Result<Self> {
        let entries = if crate::features::enterprise_flag("ZORVIA_FEATURE_FLEET") {
            match std::env::var("ZORVIA_FLEET_CONFIG") {
                Ok(path) => {
                    let text = read_config(&path)
                        .await
                        .map_err(|_| anyhow::anyhow!("Cannot read fleet registry"))?;
                    parse_enrollments(&text)?
                }
                Err(std::env::VarError::NotPresent) => Vec::new(),
                Err(_) => anyhow::bail!("Invalid fleet registry path"),
            }
        } else {
            Vec::new()
        };
        Ok(Self::new(local, namespace, entries))
    }
    fn new(local: Client, namespace: String, entries: Vec<Enrollment>) -> Self {
        let mut targets = vec![Target {
            name: "local".into(),
            namespaces: vec![namespace],
            environment: String::new(),
            region: String::new(),
            connection: Connection::Local(local),
        }];
        targets.extend(entries.into_iter().map(|entry| Target {
            name: entry.name.clone(),
            namespaces: entry.namespaces.clone(),
            environment: entry.environment.clone(),
            region: entry.region.clone(),
            connection: Connection::Remote(entry),
        }));
        Self {
            targets,
            cache: Mutex::new(None),
        }
    }
    pub async fn snapshot(&self) -> FleetSnapshot {
        let mut cache = self.cache.lock().await;
        if let Some((at, snapshot)) = &*cache {
            if at.elapsed() < CACHE_TTL {
                return snapshot.clone();
            }
        }
        let mut clusters: Vec<_> = stream::iter(self.targets.iter().cloned())
            .map(probe)
            .buffer_unordered(MAX_CLUSTERS + 1)
            .collect()
            .await;
        clusters.sort_by(|a, b| a.name.cmp(&b.name));
        let complete = clusters
            .iter()
            .filter(|c| c.vm_count.is_some() && c.node_count.is_some())
            .count();
        let snapshot = FleetSnapshot {
            observed_at: Utc::now(),
            totals: FleetTotals {
                configured_clusters: clusters.len(),
                complete_clusters: complete,
                partial: complete != clusters.len(),
                observed_vms: clusters.iter().filter_map(|c| c.vm_count).sum(),
                observed_nodes: clusters.iter().filter_map(|c| c.node_count).sum(),
            },
            clusters,
        };
        *cache = Some((Instant::now(), snapshot.clone()));
        snapshot
    }
}

fn blank(target: &Target) -> ClusterSnapshot {
    ClusterSnapshot {
        name: target.name.clone(),
        namespaces: target.namespaces.clone(),
        environment: target.environment.clone(),
        region: target.region.clone(),
        observed_at: Utc::now(),
        health: "unknown".into(),
        node_count: None,
        ready_nodes: None,
        vm_count: None,
        ready_vms: None,
        vms: None,
        issues: Vec::new(),
    }
}
async fn probe(target: Target) -> ClusterSnapshot {
    let mut result = blank(&target);
    let work = async {
        let client = match &target.connection {
            Connection::Local(client) => client.clone(),
            Connection::Remote(entry) => remote_client(entry).await.map_err(|_| ())?,
        };
        let nodes_api = Api::<Node>::all(client.clone());
        let (nodes, vms) = tokio::join!(
            list_bounded(&nodes_api, MAX_OBJECTS),
            collect_vms(client, &target.namespaces)
        );
        Ok::<_, ()>((nodes, vms))
    };
    match tokio::time::timeout(PROBE_TIMEOUT, work).await {
        Err(_) => result.issues.push("Cluster inventory timed out".into()),
        Ok(Err(())) => result
            .issues
            .push("Cluster connection configuration is unavailable or invalid".into()),
        Ok(Ok((nodes, vms))) => {
            match nodes {
                Ok(nodes) => {
                    result.node_count = Some(nodes.len());
                    result.ready_nodes = Some(
                        nodes
                            .iter()
                            .filter(|n| {
                                n.status
                                    .as_ref()
                                    .and_then(|s| s.conditions.as_ref())
                                    .is_some_and(|conditions| {
                                        conditions
                                            .iter()
                                            .any(|c| c.type_ == "Ready" && c.status == "True")
                                    })
                            })
                            .count(),
                    );
                }
                Err(issue) => result.issues.push(format!("Nodes: {issue}")),
            }
            match vms {
                Ok(vms) => {
                    result.vm_count = Some(vms.len());
                    result.ready_vms = Some(vms.iter().filter(|v| v.ready == Some(true)).count());
                    result.vms = Some(vms);
                }
                Err(issue) => result.issues.push(format!("VMs: {issue}")),
            }
            if result.issues.is_empty() {
                result.health =
                    if result.node_count == Some(0) || result.node_count != result.ready_nodes {
                        "degraded"
                    } else {
                        "healthy"
                    }
                    .into();
            }
        }
    }
    result.observed_at = Utc::now();
    result
}

fn api_issue(error: kube::Error) -> &'static str {
    // Never return raw Kubernetes errors: they can contain endpoint URLs and secrets.
    match error {
        kube::Error::Api(response) if response.code == 401 || response.code == 403 => {
            "access denied"
        }
        kube::Error::Api(response) if response.code == 404 => "resource API unavailable",
        kube::Error::Api(response) if response.code == 410 => "inventory changed; retry refresh",
        _ => "inventory unavailable",
    }
}
async fn list_bounded<T>(api: &Api<T>, limit: usize) -> Result<Vec<T>, &'static str>
where
    T: Resource + DeserializeOwned + Clone + std::fmt::Debug,
{
    let mut items = Vec::new();
    let mut token = String::new();
    let mut seen = HashSet::new();
    loop {
        let params = ListParams::default().limit(500).continue_token(&token);
        let page = api.list(&params).await.map_err(api_issue)?;
        items.extend(page.items);
        if items.len() > limit {
            return Err("inventory exceeds 5000 objects per cluster");
        }
        token = page.metadata.continue_.unwrap_or_default();
        if token.is_empty() {
            return Ok(items);
        }
        if !seen.insert(token.clone()) || seen.len() >= 32 {
            return Err("inventory pagination did not complete");
        }
    }
}
async fn collect_vms(
    client: Client,
    namespaces: &[String],
) -> Result<Vec<VmInventory>, &'static str> {
    let resource = ApiResource::from_gvk(&kube::core::GroupVersionKind::gvk(
        "kubevirt.io",
        "v1",
        "VirtualMachine",
    ));
    let mut vms = Vec::new();
    for ns in namespaces {
        let api: Api<DynamicObject> = Api::namespaced_with(client.clone(), ns, &resource);
        for vm in list_bounded(&api, MAX_OBJECTS - vms.len()).await? {
            let status = &vm.data["status"];
            vms.push(VmInventory {
                namespace: ns.clone(),
                name: vm.metadata.name.unwrap_or_default(),
                status: status["printableStatus"]
                    .as_str()
                    .unwrap_or("Unknown")
                    .into(),
                ready: status["ready"].as_bool(),
            });
        }
    }
    vms.sort_by(|a, b| (&a.namespace, &a.name).cmp(&(&b.namespace, &b.name)));
    Ok(vms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        extract::{Request, State},
        http::StatusCode,
        response::IntoResponse,
        routing::any,
        Json, Router,
    };
    use serde_json::{json, Value};
    use std::{
        collections::{HashMap, VecDeque},
        sync::Arc,
    };

    type Replies = Arc<Mutex<HashMap<String, VecDeque<(StatusCode, Value)>>>>;
    type Requests = Arc<Mutex<Vec<(String, String)>>>;
    type MockState = (Replies, Requests);
    struct Mock {
        client: Client,
        requests: Requests,
        task: tokio::task::JoinHandle<()>,
    }
    impl Drop for Mock {
        fn drop(&mut self) {
            self.task.abort();
        }
    }
    async fn handle_mock(
        State((replies, requests)): State<MockState>,
        req: Request,
    ) -> impl IntoResponse {
        requests
            .lock()
            .await
            .push((req.method().to_string(), req.uri().to_string()));
        let response = replies
            .lock()
            .await
            .get_mut(req.uri().path())
            .and_then(VecDeque::pop_front)
            .unwrap_or((
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({"message":"unexpected request", "reason":"Unexpected", "code":500}),
            ));
        (response.0, Json(response.1))
    }
    async fn mock(replies: HashMap<String, VecDeque<(StatusCode, Value)>>) -> Mock {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let state: (Replies, _) = (Arc::new(Mutex::new(replies)), requests.clone());
        let app = Router::new().fallback(any(handle_mock)).with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client =
            Client::try_from(Config::new(format!("http://{address}").parse().unwrap())).unwrap();
        Mock {
            client,
            requests,
            task,
        }
    }
    fn list(items: Vec<Value>, next: &str) -> (StatusCode, Value) {
        (
            StatusCode::OK,
            json!({"apiVersion":"v1", "kind":"List", "metadata":{"continue":next}, "items":items}),
        )
    }
    fn node(ready: &str) -> Value {
        json!({"apiVersion":"v1", "kind":"Node", "metadata":{"name":"node-1"}, "status":{"conditions":[{"type":"Ready", "status":ready}]}})
    }
    fn vm(name: &str, ready: Option<bool>) -> Value {
        json!({"apiVersion":"kubevirt.io/v1", "kind":"VirtualMachine", "metadata":{"name":name}, "status":{"ready":ready, "printableStatus":"Stopped"}})
    }
    fn replies(
        nodes: (StatusCode, Value),
        vms: (StatusCode, Value),
    ) -> HashMap<String, VecDeque<(StatusCode, Value)>> {
        HashMap::from([
            ("/api/v1/nodes".into(), VecDeque::from([nodes])),
            (
                "/apis/kubevirt.io/v1/namespaces/tenant/virtualmachines".into(),
                VecDeque::from([vms]),
            ),
        ])
    }
    fn enrollment() -> Value {
        json!({"name":"east", "kubeconfig":"/etc/fleet/east.yaml", "context":"east-readonly", "namespaces":["tenant"]})
    }
    fn parse(value: Value) -> anyhow::Result<Vec<Enrollment>> {
        parse_enrollments(&value.to_string())
    }
    #[test]
    fn enrollment_requires_unique_explicit_scopes() {
        assert_eq!(parse(json!([enrollment()])).unwrap().len(), 1);
        assert!(parse(json!([enrollment(), enrollment()])).is_err());
        for (key, value) in [
            ("name", json!("local")),
            ("name", json!("East")),
            ("kubeconfig", json!("relative")),
            ("context", json!("")),
            ("namespaces", json!([])),
            ("namespaces", json!(["tenant", "tenant"])),
            ("namespaces", json!(["*"])),
            ("environment", json!("invalid\nmetadata")),
            ("unexpected", json!(true)),
        ] {
            let mut entry = enrollment();
            entry[key] = value;
            assert!(parse(json!([entry])).is_err(), "{key}");
        }
        let mut entries = Vec::new();
        for i in 0..17 {
            let mut entry = enrollment();
            entry["name"] = json!(format!("cluster-{i}"));
            entries.push(entry);
        }
        assert!(parse(json!(entries)).is_err());
    }
    #[test]
    fn kubeconfig_rejects_plugins_file_references_and_insecure_transport() {
        let base = json!({"apiVersion":"v1", "kind":"Config", "clusters":[{"name":"east", "cluster":{"server":"https://east.example.invalid"}}], "users":[{"name":"reader", "user":{}}], "contexts":[{"name":"east-readonly", "context":{"cluster":"east", "user":"reader"}}]});
        let cfg = |value: &Value| Kubeconfig::from_yaml(&value.to_string()).unwrap();
        assert!(validate_kubeconfig(&cfg(&base)).is_ok());
        for (key, value) in [
            (
                "exec",
                json!({"apiVersion":"client.authentication.k8s.io/v1", "command":"echo"}),
            ),
            ("auth-provider", json!({"name":"gcp"})),
            ("tokenFile", json!("/sensitive")),
            ("client-certificate", json!("/cert")),
            ("client-key", json!("/key")),
        ] {
            let mut altered = base.clone();
            altered["users"][0]["user"][key] = value;
            assert!(validate_kubeconfig(&cfg(&altered)).is_err(), "{key}");
        }
        for (key, value) in [
            ("insecure-skip-tls-verify", json!(true)),
            ("certificate-authority", json!("/ca")),
            ("proxy-url", json!("https://proxy")),
            ("server", json!("http://east")),
            ("server", json!("https://user:secret@east")),
            ("server", json!("https://east?token=secret")),
        ] {
            let mut altered = base.clone();
            altered["clusters"][0]["cluster"][key] = value;
            assert!(validate_kubeconfig(&cfg(&altered)).is_err(), "{key}");
        }
    }
    #[test]
    fn error_messages_do_not_echo_remote_details() {
        let err = kube::Error::Api(
            kube::core::Status::failure("https://user:secret@internal/token", "Forbidden")
                .with_code(403)
                .boxed(),
        );
        assert_eq!(api_issue(err), "access denied");
    }
    #[tokio::test]
    async fn inventories_real_api_paths_and_coalesces_requests() {
        let server = mock(replies(
            list(vec![node("True")], ""),
            list(vec![vm("db", None), vm("api", Some(true))], ""),
        ))
        .await;
        let fleet = Arc::new(FleetService::new(
            server.client.clone(),
            "tenant".into(),
            vec![],
        ));
        let (a, b) = tokio::join!(fleet.snapshot(), fleet.snapshot());
        assert_eq!(a.observed_at, b.observed_at);
        assert_eq!(a.clusters[0].health, "healthy");
        assert_eq!(a.clusters[0].vm_count, Some(2));
        assert_eq!(a.clusters[0].ready_vms, Some(1));
        assert_eq!(a.clusters[0].vms.as_ref().unwrap()[0].name, "api");
        assert_eq!(a.clusters[0].vms.as_ref().unwrap()[1].ready, None);
        assert!(!a.totals.partial);
        let requests = server.requests.lock().await;
        assert_eq!(requests.len(), 2);
        assert!(requests.iter().all(|r| r.0 == "GET"));
        assert!(requests.iter().any(|r| r
            .1
            .starts_with("/apis/kubevirt.io/v1/namespaces/tenant/virtualmachines?")));
    }
    #[tokio::test]
    async fn denied_vm_api_preserves_nodes_and_unknown_counts() {
        let denied = (
            StatusCode::FORBIDDEN,
            json!({"code":403,"message":"secret endpoint", "reason":"Forbidden"}),
        );
        let server = mock(replies(list(vec![node("True")], ""), denied)).await;
        let snapshot = FleetService::new(server.client.clone(), "tenant".into(), vec![])
            .snapshot()
            .await;
        let cluster = &snapshot.clusters[0];
        assert_eq!(cluster.node_count, Some(1));
        assert_eq!(cluster.vm_count, None);
        assert!(cluster.vms.is_none());
        assert_eq!(cluster.health, "unknown");
        assert_eq!(cluster.issues, vec!["VMs: access denied"]);
        assert!(snapshot.totals.partial);
        assert_eq!(snapshot.totals.complete_clusters, 0);
        assert_eq!(snapshot.totals.observed_nodes, 1);
        assert!(!serde_json::to_string(&snapshot)
            .unwrap()
            .contains("secret endpoint"));
    }
    #[tokio::test]
    async fn empty_inventory_is_distinct_from_unavailable() {
        let server = mock(replies(list(vec![node("False")], ""), list(vec![], ""))).await;
        let snapshot = FleetService::new(server.client.clone(), "tenant".into(), vec![])
            .snapshot()
            .await;
        assert_eq!(snapshot.clusters[0].vm_count, Some(0));
        assert_eq!(snapshot.clusters[0].health, "degraded");
        assert!(!snapshot.totals.partial);
    }
    #[tokio::test]
    async fn pagination_collects_all_pages_and_rejects_loops_and_limits() {
        let path = "/api/v1/nodes".to_string();
        let server = mock(HashMap::from([(
            path.clone(),
            VecDeque::from([
                list(vec![node("True")], "next"),
                list(vec![node("False")], ""),
            ]),
        )]))
        .await;
        assert_eq!(
            list_bounded(&Api::<Node>::all(server.client.clone()), 5000)
                .await
                .unwrap()
                .len(),
            2
        );
        assert!(server.requests.lock().await[1].1.contains("continue=next"));
        let server = mock(HashMap::from([(
            path.clone(),
            VecDeque::from([list(vec![], "loop"), list(vec![], "loop")]),
        )]))
        .await;
        assert_eq!(
            list_bounded(&Api::<Node>::all(server.client.clone()), 5000)
                .await
                .unwrap_err(),
            "inventory pagination did not complete"
        );
        let server = mock(HashMap::from([(
            path,
            VecDeque::from([list(vec![node("True"), node("True")], "")]),
        )]))
        .await;
        assert!(list_bounded(&Api::<Node>::all(server.client.clone()), 1)
            .await
            .is_err());
    }
    #[tokio::test]
    async fn one_namespace_failure_discards_incomplete_vm_inventory() {
        let mut responses = replies(list(vec![], ""), list(vec![vm("api", Some(true))], ""));
        responses.insert(
            "/apis/kubevirt.io/v1/namespaces/second/virtualmachines".into(),
            VecDeque::from([(
                StatusCode::NOT_FOUND,
                json!({"code":404,"message":"not found", "reason":"NotFound"}),
            )]),
        );
        let server = mock(responses).await;
        let target = Target {
            name: "local".into(),
            namespaces: vec!["tenant".into(), "second".into()],
            environment: String::new(),
            region: String::new(),
            connection: Connection::Local(server.client.clone()),
        };
        let result = probe(target).await;
        assert_eq!(result.vm_count, None);
        assert!(result.vms.is_none());
        assert_eq!(result.issues, vec!["VMs: resource API unavailable"]);
    }
    #[tokio::test]
    async fn broken_remote_does_not_hide_healthy_local_cluster() {
        let server = mock(replies(list(vec![node("True")], ""), list(vec![], ""))).await;
        let mut entry = parse(json!([enrollment()])).unwrap().remove(0);
        entry.kubeconfig = "/nonexistent/zorvia-test-kubeconfig.yaml".into();
        let snapshot = FleetService::new(server.client.clone(), "tenant".into(), vec![entry])
            .snapshot()
            .await;
        assert_eq!(snapshot.clusters.len(), 2);
        assert_eq!(snapshot.clusters[0].name, "east");
        assert_eq!(snapshot.clusters[0].health, "unknown");
        assert_eq!(snapshot.clusters[1].health, "healthy");
        assert_eq!(snapshot.totals.complete_clusters, 1);
        assert!(snapshot.totals.partial);
    }
    #[tokio::test]
    async fn expired_cache_refreshes_instead_of_reusing_old_observation() {
        let mut responses = replies(list(vec![node("True")], ""), list(vec![], ""));
        responses
            .get_mut("/api/v1/nodes")
            .unwrap()
            .push_back(list(vec![node("False")], ""));
        responses
            .get_mut("/apis/kubevirt.io/v1/namespaces/tenant/virtualmachines")
            .unwrap()
            .push_back(list(vec![vm("new-vm", Some(true))], ""));
        let server = mock(responses).await;
        let fleet = FleetService::new(server.client.clone(), "tenant".into(), vec![]);
        assert_eq!(fleet.snapshot().await.clusters[0].vm_count, Some(0));
        fleet.cache.lock().await.as_mut().unwrap().0 = Instant::now() - CACHE_TTL;
        let fresh = fleet.snapshot().await;
        assert_eq!(fresh.clusters[0].vm_count, Some(1));
        assert_eq!(fresh.clusters[0].health, "degraded");
        assert_eq!(server.requests.lock().await.len(), 4);
    }

    #[tokio::test]
    async fn hanging_cluster_is_bounded_by_deadline() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new().fallback(any(|| async {
            std::future::pending::<()>().await;
            StatusCode::OK
        }));
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let server = Mock {
            client: Client::try_from(Config::new(format!("http://{address}").parse().unwrap()))
                .unwrap(),
            requests: Arc::new(Mutex::new(Vec::new())),
            task,
        };
        let snapshot = tokio::time::timeout(
            PROBE_TIMEOUT + Duration::from_secs(2),
            FleetService::new(server.client.clone(), "tenant".into(), vec![]).snapshot(),
        )
        .await
        .unwrap();
        assert_eq!(
            snapshot.clusters[0].issues,
            vec!["Cluster inventory timed out"]
        );
        assert!(snapshot.clusters[0].vm_count.is_none());
        assert!(snapshot.clusters[0].node_count.is_none());
        assert!(snapshot.totals.partial);
    }
}
