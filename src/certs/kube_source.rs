// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Kubernetes-backed [`CertSource`]. Only `tls.crt` and `ca.crt` of `kubernetes.io/tls`
//! Secrets are copied out of the API response; every other key (notably `tls.key`) is
//! dropped immediately and never inspected.

use super::{CertSource, RawSecret, SecretScan};
use anyhow::{anyhow, Result};
use k8s_openapi::api::core::v1::{Namespace, Secret};
use kube::api::{Api, DynamicObject, ListParams, PostParams};
use kube::core::{ApiResource, GroupVersionKind};
use kube::Client;

const PAGE: u32 = 200;

pub struct KubeCertSource {
    client: Client,
    /// Namespace tried when neither a cluster-wide nor a namespace listing is permitted.
    default_namespace: String,
}

impl KubeCertSource {
    pub fn new(client: Client, default_namespace: String) -> Self {
        Self {
            client,
            default_namespace,
        }
    }
}

fn is_code(e: &kube::Error, code: u16) -> bool {
    matches!(e, kube::Error::Api(s) if s.code == code)
}

fn cm_resource() -> ApiResource {
    ApiResource::from_gvk(&GroupVersionKind::gvk(
        "cert-manager.io",
        "v1",
        "Certificate",
    ))
}

/// Copy only the public certificate fields out of a Secret.
fn raw_from_secret(s: Secret) -> RawSecret {
    let take = |k: &str| s.data.as_ref().and_then(|d| d.get(k)).map(|b| b.0.clone());
    RawSecret {
        namespace: s.metadata.namespace.clone().unwrap_or_default(),
        name: s.metadata.name.clone().unwrap_or_default(),
        tls_crt: take("tls.crt"),
        ca_crt: take("ca.crt"),
    }
}

async fn list_secrets(api: &Api<Secret>, out: &mut Vec<RawSecret>) -> Result<(), kube::Error> {
    let mut lp = ListParams::default()
        .fields("type=kubernetes.io/tls")
        .limit(PAGE);
    loop {
        let page = api.list(&lp).await?;
        let next = page.metadata.continue_.clone().filter(|c| !c.is_empty());
        out.extend(page.items.into_iter().map(raw_from_secret));
        match next {
            Some(t) => lp = lp.continue_token(&t),
            None => return Ok(()),
        }
    }
}

impl CertSource for KubeCertSource {
    async fn list_tls_secrets(&self) -> Result<SecretScan> {
        let mut scan = SecretScan::default();
        match list_secrets(&Api::all(self.client.clone()), &mut scan.secrets).await {
            Ok(()) => return Ok(scan),
            Err(e) if is_code(&e, 403) => {
                scan.secrets.clear();
                scan.skipped.push(
                    "cluster-wide Secret listing is not permitted; trying per namespace".into(),
                );
            }
            Err(e) => return Err(anyhow!("listing TLS Secrets failed: {e}")),
        }
        // Per-namespace fallback: the namespaces we may list, else just the default one.
        let namespaces: Vec<String> = match Api::<Namespace>::all(self.client.clone())
            .list(&ListParams::default())
            .await
        {
            Ok(l) => l
                .items
                .into_iter()
                .filter_map(|n| n.metadata.name)
                .collect(),
            Err(_) => {
                scan.skipped.push(
                    "namespaces cannot be listed; only the Zorvia namespace is checked".into(),
                );
                vec![self.default_namespace.clone()]
            }
        };
        for ns in namespaces {
            let api: Api<Secret> = Api::namespaced(self.client.clone(), &ns);
            match list_secrets(&api, &mut scan.secrets).await {
                Ok(()) => {}
                Err(e) if is_code(&e, 403) => {
                    scan.skipped.push(format!("namespace {ns}: not permitted"))
                }
                Err(e) => scan
                    .skipped
                    .push(format!("namespace {ns}: listing failed ({e})")),
            }
        }
        Ok(scan)
    }

    async fn list_certmanager(&self) -> Result<Option<Vec<serde_json::Value>>> {
        let api: Api<DynamicObject> = Api::all_with(self.client.clone(), &cm_resource());
        match api.list(&ListParams::default()).await {
            Ok(l) => Ok(Some(
                l.items
                    .into_iter()
                    .filter_map(|o| serde_json::to_value(o).ok())
                    .collect(),
            )),
            Err(e) if is_code(&e, 404) => Ok(None),
            Err(e) => Err(anyhow!("listing cert-manager Certificates failed: {e}")),
        }
    }

    async fn get_certmanager(
        &self,
        namespace: &str,
        name: &str,
    ) -> Result<Option<serde_json::Value>> {
        let api: Api<DynamicObject> =
            Api::namespaced_with(self.client.clone(), namespace, &cm_resource());
        match api.get_opt(name).await {
            Ok(o) => Ok(o.and_then(|o| serde_json::to_value(o).ok())),
            Err(e) => Err(anyhow!("reading the Certificate failed: {e}")),
        }
    }

    async fn replace_certmanager_status(
        &self,
        namespace: &str,
        name: &str,
        object: &serde_json::Value,
    ) -> Result<()> {
        let api: Api<DynamicObject> =
            Api::namespaced_with(self.client.clone(), namespace, &cm_resource());
        let obj: DynamicObject = serde_json::from_value(object.clone())?;
        api.replace_status(name, &PostParams::default(), &obj)
            .await
            .map(|_| ())
            .map_err(|e| {
                if is_code(&e, 409) {
                    anyhow!("Conflict: the Certificate was modified")
                } else {
                    anyhow!("updating the Certificate status failed: {e}")
                }
            })
    }
}
