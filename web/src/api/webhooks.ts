// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { apiGet, apiPost, apiDelete } from './client'

/** Mirrors src/api/http_server/web/webhook_handlers.rs, backed by the real
 * crate::api::webhooks::WebhookManager -- SSRF-guarded (HTTPS + no
 * private/internal hosts), disk-persisted, and actually delivered via a
 * real HTTPS POST with retry/backoff. The secret (if set) goes in an
 * X-Zorvia-Webhook-Secret header, not an HMAC body signature. */
export type WebhookEventName =
  | 'vm.created' | 'vm.deleted' | 'vm.started' | 'vm.stopped' | 'vm.restarted' | 'vm.failed'
  | 'vm.health_changed' | 'snapshot.created' | 'snapshot.restored' | 'backup.completed'
  | 'backup.failed' | 'migration.started' | 'migration.completed' | 'alert.triggered'
  | 'alert.resolved' | string

export interface Webhook {
  id: string
  name: string
  url: string
  events: WebhookEventName[]
  enabled: boolean
  retry_count: number
  retry_delay_secs: number
  timeout_secs: number
  created_at: string
  last_triggered?: string | null
  delivery_count: number
  failure_count: number
  success_rate: number
}

export interface CreateWebhookRequest {
  name: string
  url: string
  events: WebhookEventName[]
  secret?: string
  retry_count?: number
  retry_delay_secs?: number
}

const API_BASE = '/api'

export async function listWebhooks(): Promise<Webhook[]> {
  return apiGet<Webhook[]>(`${API_BASE}/webhooks`)
}

export async function createWebhook(req: CreateWebhookRequest): Promise<Webhook> {
  return apiPost<Webhook>(`${API_BASE}/webhooks`, req)
}

export async function deleteWebhook(id: string): Promise<void> {
  return apiDelete(`${API_BASE}/webhooks/${encodeURIComponent(id)}`)
}

export async function testWebhook(id: string): Promise<{ delivered: boolean; success: boolean }> {
  return apiPost<{ delivered: boolean; success: boolean }>(`${API_BASE}/webhooks/${encodeURIComponent(id)}/test`)
}
