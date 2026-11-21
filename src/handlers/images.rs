// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

use crate::golden_images::{GoldenImageBundle, GoldenImageSpec, ImageSourceType};
use crate::tui::colors::cli as color;
use anyhow::{anyhow, Context, Result};

#[allow(clippy::too_many_arguments)]
pub fn handle_image_bundle(
    name: String,
    version: String,
    source: String,
    source_type: String,
    size: String,
    storage_class: Option<String>,
    checksum: Option<String>,
    output: Option<String>,
    format: String,
    namespace: &str,
) -> Result<()> {
    let source_type = ImageSourceType::parse(&source_type).ok_or_else(|| {
        anyhow!(
            "invalid image source type '{}'; expected http or registry",
            source_type
        )
    })?;

    let bundle = GoldenImageBundle::build(GoldenImageSpec {
        name,
        version,
        namespace: namespace.to_string(),
        source_type,
        source,
        size,
        storage_class,
        checksum,
    })?;

    let rendered = match format.to_ascii_lowercase().as_str() {
        "yaml" | "yml" => bundle.to_multi_document_yaml()?,
        "json" => serde_json::to_string_pretty(&bundle)?,
        other => {
            return Err(anyhow!(
                "unsupported image-bundle format '{}'; expected yaml or json",
                other
            ))
        }
    };

    if let Some(path) = output {
        std::fs::write(&path, rendered)
            .with_context(|| format!("failed to write image bundle '{}'", path))?;
        println!(
            "{}",
            color::success(&format!("✓ Golden image bundle written to {}", path))
        );
    } else {
        println!("{}", rendered);
    }

    eprintln!("Stable DataSource: {}/{}", namespace, bundle.image.name);
    eprintln!("Versioned DataVolume/PVC: {}", bundle.versioned_name);
    Ok(())
}

/// `zorvia guest image-upload`: detect/validate a local disk image and, unless
/// `validate_only`, upload it to a new CDI DataVolume through the upload proxy.
#[allow(clippy::too_many_arguments)]
pub async fn handle_image_upload(
    file: String,
    name: String,
    size: String,
    storage_class: Option<String>,
    format: Option<String>,
    sha256: Option<String>,
    proxy_url: Option<String>,
    proxy_ca: Option<String>,
    max_bytes: Option<u64>,
    validate_only: bool,
    namespace: &str,
) -> Result<()> {
    use crate::disk_conversion::{validate_image_header, DiskFormat, DETECT_PREFIX_LEN};
    use tokio::io::AsyncReadExt;

    let declared = match format.as_deref() {
        None => None,
        Some(f) => Some(DiskFormat::parse(f).ok_or_else(|| anyhow!("unknown image format '{f}'"))?),
    };
    let mut f = tokio::fs::File::open(&file)
        .await
        .with_context(|| format!("cannot open '{file}'"))?;
    let len = f.metadata().await?.len();
    let mut prefix = vec![0u8; DETECT_PREFIX_LEN.min(len as usize)];
    f.read_exact(&mut prefix).await?;
    let detected = validate_image_header(&prefix, declared.as_ref()).map_err(|e| anyhow!(e))?;
    println!("Format: {} ({} bytes)", detected.name(), len);
    if validate_only {
        return Ok(());
    }

    #[cfg(not(feature = "web"))]
    {
        let _ = (
            name,
            size,
            storage_class,
            sha256,
            proxy_url,
            proxy_ca,
            max_bytes,
            namespace,
        );
        anyhow::bail!("image upload requires a build with the `web` feature");
    }
    #[cfg(feature = "web")]
    {
        use crate::image_upload as iu;
        if !crate::features::allow_non_ga("image-upload") {
            anyhow::bail!("image-upload is Experimental; set ZORVIA_EXPERIMENTAL=1 to enable");
        }
        let ca = match proxy_ca.filter(|p| !p.trim().is_empty()) {
            Some(p) => {
                Some(std::fs::read(&p).with_context(|| format!("cannot read CA file '{p}'"))?)
            }
            None => None,
        };
        let url = proxy_url
            .ok_or_else(|| anyhow!("--proxy-url (or {}) is required", iu::ENV_PROXY_URL))?;
        let cfg = iu::ProxyConfig::new(&url, ca)?;
        let req = iu::UploadRequest {
            name,
            namespace: namespace.to_string(),
            size,
            storage_class,
            declared_format: declared,
            expected_sha256: sha256,
            max_bytes: max_bytes.unwrap_or_else(iu::max_bytes_from_env),
        };
        let client = crate::kube::KubeClient::new().await?;
        // Re-open so the stream starts at byte 0 (sniffing consumes the prefix again).
        let f = tokio::fs::File::open(&file).await?;
        let report = iu::upload_image(&client, req, &cfg, iu::file_stream(f), Some(len)).await?;
        println!(
            "{}",
            color::success(&format!(
                "✓ Uploaded {} bytes to DataVolume {}/{} (format {}, sha256 {}){}",
                report.bytes,
                report.namespace,
                report.name,
                report.format,
                report.sha256,
                if report.converted_to_raw_by_cdi {
                    "; CDI converts it to raw"
                } else {
                    ""
                }
            ))
        );
        Ok(())
    }
}
