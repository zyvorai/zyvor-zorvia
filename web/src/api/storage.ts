// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { apiGet } from './client'

/** Mirrors the real PersistentVolumeClaim objects listed by
 * src/api/http_server/web/storage_handlers.rs -- not a generic "storage
 * pool" abstraction (that concept doesn't exist on Kubernetes; the deleted
 * StoragePools/StorageManager/DistributedStorage pages assumed one with no
 * real backing and were not revived). */
export interface StorageVolume {
  name: string
  status: string
  capacity: string | null
  storage_class: string | null
  access_modes: string[]
  attached_vms: string[]
  created: string | null
}

const API_BASE = '/api'

export async function listStorageVolumes(): Promise<StorageVolume[]> {
  return apiGet<StorageVolume[]>(`${API_BASE}/storage/volumes`)
}
