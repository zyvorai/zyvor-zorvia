// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Terraform scaffold generator for the Zorvia HTTP API.

pub mod provider;

use anyhow::{Context, Result};
use std::path::Path;

pub const PROVIDER_DOC: &str = "docs/TERRAFORM.md";

pub fn example_main_tf(zorvia_url: &str) -> String {
    format!(
        r#"# Zorvia VM — Terraform using the HTTP provider against the Fabric API.
# See docs/TERRAFORM.md

terraform {{
  required_version = ">= 1.5.0"
  required_providers {{
    http-full = {{
      source  = "salrashid123/http-full"
      version = ">= 1.3.0"
    }}
  }}
}}

variable "zorvia_url" {{
  type        = string
  description = "Zorvia API base (https://HOST:30152)"
  default     = "{url}"
}}

variable "zorvia_token" {{
  type        = string
  description = "JWT or API key from POST /api/v1/auth/login"
  sensitive   = true
}}

variable "vm_name" {{
  type    = string
  default = "tf-web-01"
}}

resource "terraform_data" "zorvia_vm" {{
  input = {{
    name   = var.vm_name
    image  = "quay.io/containerdisks/ubuntu:24.04"
    cpus   = 2
    memory = 2048
  }}

  provisioner "local-exec" {{
    command = <<-EOT
      curl -sk -X POST "$ZORVIA_URL/api/vms" \
        -H "Authorization: Bearer $ZORVIA_TOKEN" \
        -H "Content-Type: application/json" \
        -d '{{"name":"{name}","image":"quay.io/containerdisks/ubuntu:24.04","cpus":2,"memory":2048,"start":true}}'
    EOT
    environment = {{
      ZORVIA_URL   = var.zorvia_url
      ZORVIA_TOKEN = var.zorvia_token
    }}
  }}

  provisioner "local-exec" {{
    when    = destroy
    command = "curl -sk -X DELETE \"$ZORVIA_URL/api/vms/$VM\" -H \"Authorization: Bearer $ZORVIA_TOKEN\""
    environment = {{
      ZORVIA_URL   = var.zorvia_url
      ZORVIA_TOKEN = var.zorvia_token
      VM           = var.vm_name
    }}
  }}
}}
"#,
        url = zorvia_url.trim_end_matches('/'),
        name = "${var.vm_name}"
    )
}

pub fn example_variables_tf() -> String {
    r#"variable "zorvia_url" {
  type        = string
  description = "Zorvia API base URL"
}

variable "zorvia_token" {
  type        = string
  sensitive   = true
}

variable "vm_name" {
  type    = string
  default = "tf-web-01"
}
"#
    .to_string()
}

pub fn example_outputs_tf() -> String {
    r#"output "vm_name" {
  value = var.vm_name
}

output "console" {
  value = "${var.zorvia_url}/app/vms/${var.vm_name}/console"
}
"#
    .to_string()
}

pub fn write_scaffold(dir: &Path, zorvia_url: &str) -> Result<Vec<String>> {
    std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    let files = [
        ("main.tf", example_main_tf(zorvia_url)),
        ("variables.tf", example_variables_tf()),
        ("outputs.tf", example_outputs_tf()),
        ("terraform.tfvars.example", "zorvia_url = \"https://127.0.0.1:30152\"\nzorvia_token = \"replace-me\"\nvm_name = \"tf-web-01\"\n".into()),
        ("schema.json", provider::provider_schema_pretty()?),
    ];
    let mut written = Vec::new();
    for (name, body) in files {
        let path = dir.join(name);
        std::fs::write(&path, body).with_context(|| format!("write {}", path.display()))?;
        written.push(path.display().to_string());
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_tf_mentions_api() {
        let tf = example_main_tf("https://lab:30152");
        assert!(tf.contains("/api/vms"));
        assert!(tf.contains("30152"));
        assert!(tf.contains("Authorization"));
    }

    #[test]
    fn scaffold_writes_files() {
        let dir = std::env::temp_dir().join(format!("zorvia-tf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let files = write_scaffold(&dir, "https://example:30152").unwrap();
        assert_eq!(files.len(), 5);
        assert!(dir.join("main.tf").exists());
        assert!(dir.join("schema.json").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
