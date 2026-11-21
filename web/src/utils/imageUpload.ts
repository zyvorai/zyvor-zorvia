// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

/** The validate endpoint only needs the first ~33 KiB (docs/IMAGE_UPLOAD.md). */
export const VALIDATE_HEAD_BYTES = 33 * 1024

export const IMAGE_FORMATS = ['qcow2', 'vmdk', 'vhdx', 'vhd', 'vdi', 'iso', 'raw'] as const

const DNS_LABEL = /^[a-z0-9]([-a-z0-9]*[a-z0-9])?$/
const SIZE_QUANTITY = /^[1-9]\d*(Ki|Mi|Gi|Ti|K|M|G|T)?$/
const SHA256 = /^[0-9a-fA-F]{64}$/

/** First problem with the upload form, or null when it can be submitted. */
export function uploadFormProblem(f: { name: string; size: string; sha256: string; hasFile: boolean }): string | null {
  if (!f.hasFile) return 'Choose an image file.'
  if (!f.name) return 'Enter a name for the DataVolume.'
  if (f.name.length > 63 || !DNS_LABEL.test(f.name)) {
    return 'Name must be a DNS label: lowercase letters, digits and "-", at most 63 characters.'
  }
  if (!SIZE_QUANTITY.test(f.size)) return 'PVC size must look like 20Gi (a whole number plus Ki, Mi, Gi or Ti).'
  if (f.sha256 && !SHA256.test(f.sha256)) return 'SHA-256 must be 64 hex characters.'
  return null
}

export function uploadQuery(o: {
  size: string
  namespace?: string
  storageClass?: string
  format?: string
  sha256?: string
}): string {
  const p = new URLSearchParams({ size: o.size })
  if (o.namespace) p.set('namespace', o.namespace)
  if (o.storageClass) p.set('storage_class', o.storageClass)
  if (o.format) p.set('format', o.format)
  if (o.sha256) p.set('sha256', o.sha256.toLowerCase())
  return p.toString()
}

/** Plain-language message for an upload failure status/code. The size cap is
    server-side (ZORVIA_IMAGE_UPLOAD_MAX_BYTES, default 20 GiB) and not exposed
    by the API, so the message names the default rather than a live value. */
export function uploadErrorMessage(status: number, code: string | null, serverMessage: string): string {
  const detail = serverMessage ? ` ${serverMessage}` : ''
  switch (code) {
    case 'IMAGE_TOO_LARGE':
      return `Image is larger than the server's upload cap (20 GiB unless ZORVIA_IMAGE_UPLOAD_MAX_BYTES was changed). The upload was aborted and the DataVolume removed.${detail}`
    case 'CHECKSUM_MISMATCH':
      return `The SHA-256 you gave does not match the bytes received; nothing was kept.${detail}`
    case 'INVALID_IMAGE':
      return `The file is not a valid image of the declared format.${detail}`
    case 'UPLOAD_PROXY_NOT_CONFIGURED':
      return `The server has no CDI upload proxy configured (ZORVIA_CDI_UPLOAD_PROXY_URL).${detail}`
    case 'UPLOAD_FAILED':
      return `The CDI upload proxy rejected or dropped the upload; the DataVolume was removed.${detail}`
    default:
      break
  }
  if (status === 413) {
    // A reverse proxy in front of Zorvia can answer 413 without a JSON body.
    return 'The upload was refused as too large (413). Check the server cap and any reverse-proxy body limit.'
  }
  return serverMessage || `Upload failed (HTTP ${status}).`
}

export function formatPercent(loaded: number, total: number): number {
  if (!total || total <= 0) return 0
  return Math.max(0, Math.min(100, Math.floor((loaded / total) * 100)))
}
