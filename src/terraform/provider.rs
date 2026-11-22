// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Terraform provider schema for a future `hashicorp/zorvia` publish.
//!
//! This is the contract the HTTP API already implements. A Go/Rust provider
//! binary can load this JSON instead of drifting from the control plane.

use serde_json::{json, Value};

pub fn provider_schema() -> Value {
    json!({
        "provider": {
            "name": "zorvia",
            "version": "0.3.0",
            "source": "zyvorai/zorvia",
            "protocol": { "create": "POST", "read": "GET", "update": "PATCH", "delete": "DELETE" },
            "config": {
                "url": { "type": "string", "required": true, "description": "Zorvia API base URL" },
                "token": { "type": "string", "required": true, "sensitive": true },
                "insecure": { "type": "bool", "required": false, "description": "Skip TLS verify (lab only)" }
            }
        },
        "resources": {
            "zorvia_vm": {
                "path": "/api/vms",
                "id": "name",
                "schema": {
                    "name": { "type": "string", "required": true, "force_new": true },
                    "image": { "type": "string", "required": true },
                    "cpus": { "type": "number", "required": true },
                    "memory": { "type": "number", "required": true, "description": "MiB" },
                    "disk": { "type": "number", "required": false },
                    "start": { "type": "bool", "required": false, "default": true },
                    "expose_ssh": { "type": "bool", "required": false },
                    "expose_vnc": { "type": "bool", "required": false },
                    "guest_os": { "type": "string", "required": false }
                },
                "endpoints": {
                    "create": "POST /api/vms",
                    "read": "GET /api/vms/{name}",
                    "delete": "DELETE /api/vms/{name}",
                    "start": "POST /api/vms/{name}/start",
                    "stop": "POST /api/vms/{name}/stop",
                    "pause": "POST /api/vms/{name}/pause",
                    "resume": "POST /api/vms/{name}/resume"
                }
            },
            "zorvia_snapshot": {
                "path": "/api/vms/{vm}/snapshots",
                "schema": {
                    "vm": { "type": "string", "required": true, "force_new": true },
                    "name": { "type": "string", "required": true, "force_new": true },
                    "description": { "type": "string", "required": false }
                }
            },
            "zorvia_image_download": {
                "path": "/api/images/cloud/download",
                "schema": {
                    "name": { "type": "string", "required": true }
                },
                "endpoints": {
                    "create": "POST /api/images/cloud/download",
                    "read": "GET /api/images/downloads"
                }
            }
        }
    })
}

pub fn provider_schema_pretty() -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(&provider_schema())?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_has_vm_and_auth() {
        let s = provider_schema();
        assert_eq!(s["provider"]["name"], "zorvia");
        assert_eq!(s["resources"]["zorvia_vm"]["path"], "/api/vms");
        assert!(s["provider"]["config"]["token"]["sensitive"]
            .as_bool()
            .unwrap());
        assert!(s["resources"]["zorvia_vm"]["endpoints"]["pause"]
            .as_str()
            .unwrap()
            .contains("/pause"));
    }
}
