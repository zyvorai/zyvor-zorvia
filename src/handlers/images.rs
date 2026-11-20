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
