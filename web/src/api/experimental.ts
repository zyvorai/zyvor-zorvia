// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { apiFetch } from './client'
import { formatHttpErrorBody } from '../utils/apiError'
import { parseJsonResponse } from '../utils/parseJsonResponse'
import { ExperimentalDisabledError, experimentalDisabledMessage } from '../utils/experimental'

/** apiFetch that turns a 403 feature-disabled answer into ExperimentalDisabledError
    and any other failure into an Error with a readable message. */
export async function featureFetch(url: string, init?: RequestInit): Promise<Response> {
  const res = await apiFetch(url, init)
  if (res.ok) return res
  const body = await res.text().catch(() => '')
  const disabled = experimentalDisabledMessage(res.status, body)
  if (disabled) throw new ExperimentalDisabledError(disabled)
  throw new Error(formatHttpErrorBody(res.status, res.statusText, body))
}

export async function featureJson<T>(url: string, method = 'GET', body?: unknown): Promise<T> {
  const res = await featureFetch(url, {
    method,
    headers: body !== undefined ? { 'Content-Type': 'application/json' } : undefined,
    body: body !== undefined ? JSON.stringify(body) : undefined,
  })
  return parseJsonResponse<T>(res)
}
