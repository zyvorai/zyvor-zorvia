// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

/** Error codes the API uses when an Experimental feature is switched off
    (403). Image upload and volume encryption answer EXPERIMENTAL_DISABLED;
    DR pairs answer feature_disabled (they need a second flag). */
const DISABLED_CODES = new Set(['experimental_disabled', 'feature_disabled'])

export class ExperimentalDisabledError extends Error {
  constructor(message: string) {
    super(message)
    this.name = 'ExperimentalDisabledError'
  }
}

export function isExperimentalDisabled(e: unknown): e is ExperimentalDisabledError {
  return e instanceof ExperimentalDisabledError
}

interface ErrorBody {
  error?: string | { code?: string; message?: string }
  error_code?: string
  message?: string
}

/** Machine error code of an API error body, if any. Understands the flat
    `{error, error_code}` and the nested `{error: {code, message}}` shapes. */
export function apiErrorCode(bodyText: string): string | null {
  try {
    const j = JSON.parse(bodyText) as ErrorBody
    if (typeof j.error_code === 'string') return j.error_code
    if (typeof j.error === 'object' && j.error && typeof j.error.code === 'string') return j.error.code
    if (typeof j.error === 'string') return j.error
  } catch {
    /* not JSON */
  }
  return null
}

/** Returns the server's message when (status, body) is a 403 feature-disabled
    answer, otherwise null. */
export function experimentalDisabledMessage(status: number, bodyText: string): string | null {
  if (status !== 403) return null
  const code = apiErrorCode(bodyText)
  if (!code || !DISABLED_CODES.has(code.toLowerCase())) return null
  try {
    const j = JSON.parse(bodyText) as ErrorBody
    const nested = typeof j.error === 'object' && j.error !== null ? j.error : undefined
    return nested?.message ?? j.message ?? 'This feature is experimental and disabled on this server.'
  } catch {
    return 'This feature is experimental and disabled on this server.'
  }
}
