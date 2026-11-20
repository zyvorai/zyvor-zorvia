// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Rook-Ceph integration: bootstrapping the Rook operator, provisioning
//! `CephCluster`/`CephBlockPool`/`CephFilesystem`/`CephObjectStore`, and
//! wiring `StorageClass`/`VolumeSnapshotClass` objects to Rook's CSI
//! provisioners.
//!
//! Rook CRDs are treated as raw JSON manifests (`crate::rook::manifests`)
//! rather than fully typed `kube::CustomResource` structs — their schemas
//! are large and this crate only needs a curated subset of fields, the same
//! tradeoff `crate::kube::cdi` makes for CDI DataVolumes.

pub mod client;
pub mod health;
pub mod manifests;

pub use client::{BootstrapOptions, BootstrapReport, RookClient};
pub use health::{CephHealthState, CephHealthSummary};
pub use manifests::{CephBlockPoolSpec, CephClusterSpec, CephFilesystemSpec, CephObjectStoreSpec};
