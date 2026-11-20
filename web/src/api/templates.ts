// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { apiGet, apiPost } from './client'

/** Mirrors src/api/http_server/web/template_handlers.rs, backed by the
 * real crate::templates::TEMPLATES catalog (~30 curated OS VMConfigs).
 * Browse-only for now -- deploying a template into a real VM create
 * request isn't wired yet. */
export interface TemplateDetail {
  name: string
  cpu: { cores: number; sockets: number; threads: number; model?: string | null }
  memory: Record<string, unknown>
  firmware?: Record<string, unknown> | null
  features?: Record<string, unknown> | null
  eviction_strategy?: string | null
  enable_tpm: boolean
  enable_rng: boolean
  machine_type?: string | null
  [key: string]: unknown
}

const API_BASE = '/api'

export async function listTemplatesByFamily(): Promise<Record<string, string[]>> {
  return apiGet<Record<string, string[]>>(`${API_BASE}/templates`)
}

export async function getTemplate(name: string): Promise<TemplateDetail> {
  return apiGet<TemplateDetail>(`${API_BASE}/templates/${encodeURIComponent(name)}`)
}

/** Deploys a template for real: each template is already a complete,
 * bootable VM config (real container-disk image + cloud-init), so this
 * just calls the same real create-VM path with the template's config. */
export async function deployTemplate(templateName: string, vmName: string, start = true): Promise<{ vm_name: string; template: string }> {
  return apiPost(`${API_BASE}/templates/${encodeURIComponent(templateName)}/deploy`, { vm_name: vmName, start })
}
