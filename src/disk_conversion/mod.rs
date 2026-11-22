// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

// Disk Conversion - Disk format conversion utilities

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskConverter {
    pub supported_formats: Vec<DiskFormat>,
    pub conversions: Vec<ConversionJob>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DiskFormat {
    Raw,
    Qcow2,
    Vmdk,
    Vdi,
    Vhd,
    Vhdx,
    Iso,
}

impl DiskFormat {
    pub fn extension(&self) -> &str {
        match self {
            Self::Raw => "img",
            Self::Qcow2 => "qcow2",
            Self::Vmdk => "vmdk",
            Self::Vdi => "vdi",
            Self::Vhd => "vhd",
            Self::Vhdx => "vhdx",
            Self::Iso => "iso",
        }
    }

    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            "img" | "raw" => Some(Self::Raw),
            "qcow2" => Some(Self::Qcow2),
            "vmdk" => Some(Self::Vmdk),
            "vdi" => Some(Self::Vdi),
            "vhd" => Some(Self::Vhd),
            "vhdx" => Some(Self::Vhdx),
            "iso" => Some(Self::Iso),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversionJob {
    pub id: String,
    pub source_path: String,
    pub target_path: String,
    pub source_format: DiskFormat,
    pub target_format: DiskFormat,
    pub status: ConversionStatus,
    pub progress_percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ConversionStatus {
    Pending,
    InProgress,
    Completed,
    Failed(String),
}

impl DiskConverter {
    pub fn new() -> Self {
        Self {
            supported_formats: vec![
                DiskFormat::Raw,
                DiskFormat::Qcow2,
                DiskFormat::Vmdk,
                DiskFormat::Vdi,
                DiskFormat::Vhd,
            ],
            conversions: Vec::new(),
        }
    }

    pub fn can_convert(&self, from: &DiskFormat, to: &DiskFormat) -> bool {
        self.supported_formats.contains(from) && self.supported_formats.contains(to) && from != to
    }

    pub fn start_conversion(
        &mut self,
        source: &str,
        target: &str,
        from: DiskFormat,
        to: DiskFormat,
    ) -> Option<String> {
        if !self.can_convert(&from, &to) {
            return None;
        }
        let id = format!("conv-{}", chrono::Utc::now().timestamp_micros());
        self.conversions.push(ConversionJob {
            id: id.clone(),
            source_path: source.to_string(),
            target_path: target.to_string(),
            source_format: from,
            target_format: to,
            status: ConversionStatus::Pending,
            progress_percent: 0.0,
        });
        Some(id)
    }

    pub fn get_job(&self, id: &str) -> Option<&ConversionJob> {
        self.conversions.iter().find(|j| j.id == id)
    }
    pub fn active_jobs(&self) -> Vec<&ConversionJob> {
        self.conversions
            .iter()
            .filter(|j| {
                matches!(
                    j.status,
                    ConversionStatus::InProgress | ConversionStatus::Pending
                )
            })
            .collect()
    }
}

impl Default for DiskConverter {
    fn default() -> Self {
        Self::new()
    }
}

/// Bytes of leading data `detect_format` needs to recognise every supported
/// format (the ISO 9660 signature sits at offset 0x8001).
pub const DETECT_PREFIX_LEN: usize = 0x8006;

impl DiskFormat {
    /// Lowercase name as used by `qemu-img -f` and the upload API.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Qcow2 => "qcow2",
            Self::Vmdk => "vmdk",
            Self::Vdi => "vdi",
            Self::Vhd => "vhd",
            Self::Vhdx => "vhdx",
            Self::Iso => "iso",
        }
    }

    /// Parse a user-supplied format name (`raw`, `img`, `qcow2`, ...).
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "raw" => Some(Self::Raw),
            other => Self::from_extension(other),
        }
    }
}

/// Identify a disk image from its leading bytes (magic numbers only; the
/// file is never executed or passed to another tool). Returns `None` when no
/// known signature matches, which for a headerless raw disk is the normal
/// case: raw cannot be detected, only declared.
pub fn detect_format(prefix: &[u8]) -> Option<DiskFormat> {
    if prefix.starts_with(b"QFI\xfb") {
        return Some(DiskFormat::Qcow2);
    }
    // Monolithic sparse VMDK header, or a text descriptor file.
    if prefix.starts_with(b"KDMV") || prefix.starts_with(b"# Disk DescriptorFile") {
        return Some(DiskFormat::Vmdk);
    }
    if prefix.starts_with(b"vhdxfile") {
        return Some(DiskFormat::Vhdx);
    }
    // Dynamic/differencing VHD carry a copy of the "conectix" footer first.
    if prefix.starts_with(b"conectix") {
        return Some(DiskFormat::Vhd);
    }
    // VDI: 0xbeda107f little-endian at offset 64.
    if prefix.len() >= 68 && prefix[64..68] == [0x7f, 0x10, 0xda, 0xbe] {
        return Some(DiskFormat::Vdi);
    }
    // ISO 9660 primary volume descriptor: "CD001" at 0x8001.
    if prefix.len() >= 0x8006 && &prefix[0x8001..0x8006] == b"CD001" {
        return Some(DiskFormat::Iso);
    }
    None
}

/// Decide the format of an image about to be uploaded. A detected signature
/// must agree with the declared format; an undetected image is accepted only
/// when it is explicitly declared `raw`, so an arbitrary file is never
/// silently treated as a disk.
pub fn validate_image_header(
    prefix: &[u8],
    declared: Option<&DiskFormat>,
) -> Result<DiskFormat, String> {
    match (detect_format(prefix), declared) {
        (Some(found), Some(want)) if &found != want => Err(format!(
            "image looks like {} but was declared {}",
            found.name(),
            want.name()
        )),
        (Some(found), _) => Ok(found),
        (None, Some(DiskFormat::Raw)) => Ok(DiskFormat::Raw),
        (None, Some(want)) => Err(format!("image does not carry a {} signature", want.name())),
        (None, None) => Err(
            "unrecognised image format; declare raw explicitly if this is a headerless raw disk"
                .to_string(),
        ),
    }
}

#[cfg(test)]
mod detect_tests {
    use super::*;

    fn padded(prefix: &[u8], len: usize) -> Vec<u8> {
        let mut v = vec![0u8; len];
        v[..prefix.len()].copy_from_slice(prefix);
        v
    }

    #[test]
    fn detects_each_signature() {
        assert_eq!(
            detect_format(&padded(b"QFI\xfb", 512)),
            Some(DiskFormat::Qcow2)
        );
        assert_eq!(detect_format(&padded(b"KDMV", 512)), Some(DiskFormat::Vmdk));
        assert_eq!(
            detect_format(b"# Disk DescriptorFile\nversion=1"),
            Some(DiskFormat::Vmdk)
        );
        assert_eq!(
            detect_format(&padded(b"vhdxfile", 512)),
            Some(DiskFormat::Vhdx)
        );
        assert_eq!(
            detect_format(&padded(b"conectix", 512)),
            Some(DiskFormat::Vhd)
        );
        let mut vdi = vec![0u8; 512];
        vdi[64..68].copy_from_slice(&[0x7f, 0x10, 0xda, 0xbe]);
        assert_eq!(detect_format(&vdi), Some(DiskFormat::Vdi));
        let mut iso = vec![0u8; DETECT_PREFIX_LEN];
        iso[0x8001..0x8006].copy_from_slice(b"CD001");
        assert_eq!(detect_format(&iso), Some(DiskFormat::Iso));
        assert_eq!(detect_format(&[0u8; 4096]), None);
        assert_eq!(detect_format(b""), None);
    }

    #[test]
    fn validation_requires_agreement_and_explicit_raw() {
        let q = padded(b"QFI\xfb", 512);
        assert_eq!(validate_image_header(&q, None), Ok(DiskFormat::Qcow2));
        assert_eq!(
            validate_image_header(&q, Some(&DiskFormat::Qcow2)),
            Ok(DiskFormat::Qcow2)
        );
        assert!(validate_image_header(&q, Some(&DiskFormat::Vmdk)).is_err());
        let zeros = [0u8; 1024];
        assert!(validate_image_header(&zeros, None).is_err());
        assert!(validate_image_header(&zeros, Some(&DiskFormat::Qcow2)).is_err());
        assert_eq!(
            validate_image_header(&zeros, Some(&DiskFormat::Raw)),
            Ok(DiskFormat::Raw)
        );
    }

    #[test]
    fn format_names_round_trip() {
        for f in [
            DiskFormat::Raw,
            DiskFormat::Qcow2,
            DiskFormat::Vmdk,
            DiskFormat::Vdi,
            DiskFormat::Vhd,
            DiskFormat::Vhdx,
            DiskFormat::Iso,
        ] {
            assert_eq!(DiskFormat::parse(f.name()), Some(f));
        }
        assert_eq!(DiskFormat::parse("IMG"), Some(DiskFormat::Raw));
        assert_eq!(DiskFormat::parse("exe"), None);
    }
}
