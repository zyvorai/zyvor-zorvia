// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { apiFetch, getToken } from './client'
import { featureFetch } from './experimental'
import { ExperimentalDisabledError, apiErrorCode, experimentalDisabledMessage } from '../utils/experimental'
import { VALIDATE_HEAD_BYTES, uploadErrorMessage, uploadQuery } from '../utils/imageUpload'
import { parseJsonResponse } from '../utils/parseJsonResponse'

const API_BASE = '/api'

export interface ValidateResult {
  valid: boolean
  format?: string
  detected?: string | null
  converted_to_raw_by_cdi?: boolean
  error?: string
}

export interface UploadResult {
  name: string
  format: string
  bytes: number
  sha256: string
}

export interface UploadOptions {
  size: string
  namespace?: string
  storageClass?: string
  format?: string
  sha256?: string
  onProgress?: (loaded: number, total: number) => void
  signal?: AbortSignal
  /** Server cap, used only to word a too-large error. */
  maxBytes?: number | null
}

/** Header-only check: sends at most the first ~33 KiB (docs/IMAGE_UPLOAD.md). */
export async function validateImage(file: Blob, format?: string): Promise<ValidateResult> {
  const q = format ? `?format=${encodeURIComponent(format)}` : ''
  const res = await featureFetch(`${API_BASE}/images/validate${q}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/octet-stream' },
    body: file.slice(0, VALIDATE_HEAD_BYTES),
  })
  return parseJsonResponse<ValidateResult>(res)
}

/** Probe for the experimental gate without sending any image: an empty
    validate request is answered 403 when the feature is off. Returns the
    server message when disabled, otherwise null. Other failures are ignored
    (the page still works; the real request will surface them). */
export async function probeImageUploadDisabled(): Promise<string | null> {
  const res = await apiFetch(`${API_BASE}/images/validate`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/octet-stream' },
    body: new Blob([]),
  }).catch(() => null)
  if (!res || res.ok) return null
  return experimentalDisabledMessage(res.status, await res.text().catch(() => ''))
}

/** Streams the file to POST /api/images/uploads/{name}. XHR rather than fetch
    because fetch cannot report upload progress. Progress counts bytes handed
    to Zorvia; the server then streams them on to CDI, so 100% means "sent",
    and the request resolves only once CDI has accepted the image. */
export function uploadImage(name: string, file: File, opts: UploadOptions): Promise<UploadResult> {
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest()
    xhr.open('POST', `${API_BASE}/images/uploads/${encodeURIComponent(name)}?${uploadQuery(opts)}`)
    const token = getToken()
    if (token) xhr.setRequestHeader('Authorization', `Bearer ${token}`)
    xhr.setRequestHeader('Content-Type', 'application/octet-stream')
    xhr.upload.onprogress = (e) => {
      if (e.lengthComputable) opts.onProgress?.(e.loaded, e.total)
    }
    xhr.onerror = () => reject(new Error('Network error during upload. The server may have aborted it (size cap or timeout).'))
    xhr.onabort = () => reject(new Error('Upload cancelled.'))
    xhr.onload = () => {
      const body = xhr.responseText ?? ''
      if (xhr.status >= 200 && xhr.status < 300) {
        try {
          resolve(JSON.parse(body) as UploadResult)
        } catch {
          reject(new Error('Upload finished but the response was not valid JSON.'))
        }
        return
      }
      const disabled = experimentalDisabledMessage(xhr.status, body)
      if (disabled) return reject(new ExperimentalDisabledError(disabled))
      let msg = ''
      try {
        const j = JSON.parse(body) as { error?: string | { message?: string }; message?: string }
        msg = typeof j.error === 'object' ? (j.error?.message ?? '') : (j.message ?? '')
      } catch {
        /* non-JSON body */
      }
      reject(new Error(uploadErrorMessage(xhr.status, apiErrorCode(body), msg, opts.maxBytes)))
    }
    opts.signal?.addEventListener('abort', () => xhr.abort())
    xhr.send(file)
  })
}

export interface UploadConfig {
  max_bytes: number
  default_max_bytes: number
}

/** The server's effective upload size cap, or null when it cannot be read. */
export async function getUploadConfig(): Promise<UploadConfig | null> {
  const res = await apiFetch(`${API_BASE}/images/upload-config`).catch(() => null)
  if (!res || !res.ok) return null
  const j = (await res.json().catch(() => null)) as Partial<UploadConfig> | null
  return j && typeof j.max_bytes === 'number' && j.max_bytes > 0 ? (j as UploadConfig) : null
}

export interface UploadStatus {
  name: string
  namespace: string
  phase: string
  progress_percent?: number | null
}

export async function getUploadStatus(name: string, namespace?: string): Promise<UploadStatus> {
  const q = namespace ? `?namespace=${encodeURIComponent(namespace)}` : ''
  const res = await featureFetch(`${API_BASE}/images/uploads/${encodeURIComponent(name)}${q}`)
  return parseJsonResponse<UploadStatus>(res)
}
