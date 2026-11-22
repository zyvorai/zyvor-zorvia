// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { describe, it, expect } from 'vitest'
import { apiErrorCode, experimentalDisabledMessage } from './experimental'
import { formatCap, formatPercent, uploadErrorMessage, uploadFormProblem, uploadQuery } from './imageUpload'
import { canConfirmFailover, formatDuration, healthBadge, lagPercent, pairFormProblem, type PairForm } from './drPairs'
import {
  encryptFormProblem,
  rotateFormProblem,
  rotateSummary,
  toEncryptRequest,
  type EncryptForm,
} from './volumeEncryption'

const nested = (code: string, message: string) => JSON.stringify({ success: false, error: { code, message } })

describe('experimental gating', () => {
  it('recognises EXPERIMENTAL_DISABLED and feature_disabled 403s', () => {
    expect(experimentalDisabledMessage(403, nested('EXPERIMENTAL_DISABLED', 'image-upload is Experimental'))).toBe(
      'image-upload is Experimental',
    )
    expect(experimentalDisabledMessage(403, nested('feature_disabled', 'DR replication is disabled'))).toBe(
      'DR replication is disabled',
    )
  })
  it('ignores other 403s, other statuses and non-JSON', () => {
    expect(experimentalDisabledMessage(403, nested('forbidden', 'nope'))).toBeNull()
    expect(experimentalDisabledMessage(500, nested('EXPERIMENTAL_DISABLED', 'x'))).toBeNull()
    expect(experimentalDisabledMessage(403, '<html>')).toBeNull()
  })
  it('reads codes from flat and nested bodies', () => {
    expect(apiErrorCode('{"error":"X","error_code":"IMAGE_TOO_LARGE"}')).toBe('IMAGE_TOO_LARGE')
    expect(apiErrorCode(nested('CHECKSUM_MISMATCH', 'm'))).toBe('CHECKSUM_MISMATCH')
    expect(apiErrorCode('plain')).toBeNull()
  })
})

describe('image upload helpers', () => {
  const ok = { name: 'ubuntu-24', size: '20Gi', sha256: '', hasFile: true }
  it('accepts a good form and rejects bad fields', () => {
    expect(uploadFormProblem(ok)).toBeNull()
    expect(uploadFormProblem({ ...ok, hasFile: false })).toMatch(/Choose/)
    expect(uploadFormProblem({ ...ok, name: '' })).toMatch(/name/i)
    expect(uploadFormProblem({ ...ok, name: 'Bad_Name' })).toMatch(/DNS label/)
    expect(uploadFormProblem({ ...ok, name: 'a'.repeat(64) })).toMatch(/DNS label/)
    expect(uploadFormProblem({ ...ok, size: '20 GB' })).toMatch(/PVC size/)
    expect(uploadFormProblem({ ...ok, size: '0Gi' })).toMatch(/PVC size/)
    expect(uploadFormProblem({ ...ok, sha256: 'abc' })).toMatch(/SHA-256/)
    expect(uploadFormProblem({ ...ok, sha256: 'a'.repeat(64) })).toBeNull()
  })
  it('builds the query string with only the set options', () => {
    expect(uploadQuery({ size: '20Gi' })).toBe('size=20Gi')
    const q = new URLSearchParams(
      uploadQuery({ size: '10Gi', namespace: 'vms', storageClass: 'ceph', format: 'raw', sha256: 'AB'.repeat(32) }),
    )
    expect(q.get('namespace')).toBe('vms')
    expect(q.get('storage_class')).toBe('ceph')
    expect(q.get('format')).toBe('raw')
    expect(q.get('sha256')).toBe('ab'.repeat(32))
  })
  it('explains the size cap and other server errors', () => {
    expect(uploadErrorMessage(413, 'IMAGE_TOO_LARGE', '')).toMatch(/20 GiB/)
    expect(uploadErrorMessage(413, 'IMAGE_TOO_LARGE', '', 5 * 1024 ** 3)).toMatch(/5 GiB\)/)
    expect(uploadErrorMessage(413, 'IMAGE_TOO_LARGE', '', 1536 * 1024 ** 2)).toMatch(/1\.5 GiB/)
    expect(formatCap(20 * 1024 ** 3)).toBe('20 GiB')
    expect(uploadErrorMessage(413, null, '')).toMatch(/reverse-proxy/)
    expect(uploadErrorMessage(422, 'CHECKSUM_MISMATCH', '')).toMatch(/SHA-256/)
    expect(uploadErrorMessage(400, 'INVALID_IMAGE', 'declared qcow2')).toMatch(/declared qcow2/)
    expect(uploadErrorMessage(503, 'UPLOAD_PROXY_NOT_CONFIGURED', '')).toMatch(/ZORVIA_CDI_UPLOAD_PROXY_URL/)
    expect(uploadErrorMessage(502, 'UPLOAD_FAILED', '')).toMatch(/removed/)
    expect(uploadErrorMessage(500, null, '')).toBe('Upload failed (HTTP 500).')
  })
  it('clamps progress', () => {
    expect(formatPercent(50, 200)).toBe(25)
    expect(formatPercent(300, 200)).toBe(100)
    expect(formatPercent(1, 0)).toBe(0)
  })
})

describe('DR pair helpers', () => {
  const form: PairForm = { name: 'web', vm: 'web-1', namespace: '', drNamespace: '', rpoSeconds: '3600', rtoSeconds: '' }
  it('formats durations', () => {
    expect(formatDuration(0)).toBe('0s')
    expect(formatDuration(59)).toBe('59s')
    expect(formatDuration(3725)).toBe('1h 2m')
    expect(formatDuration(90000)).toBe('1d 1h')
    expect(formatDuration(null)).toBe('—')
    expect(formatDuration(-1)).toBe('—')
  })
  it('maps every health value to a badge', () => {
    expect(healthBadge('healthy').status).toBe('healthy')
    expect(healthBadge('rpo_breached')).toEqual({ status: 'failed', label: 'RPO breached' })
    expect(healthBadge('failed_over').label).toBe('Failed over')
    expect(healthBadge('no_backup_yet').status).toBe('pending')
  })
  it('caps lag percent at 100', () => {
    expect(lagPercent({ lag_seconds: 1800, rpo_seconds: 3600 })).toBe(50)
    expect(lagPercent({ lag_seconds: 9000, rpo_seconds: 3600 })).toBe(100)
  })
  it('validates the create form against the documented RPO bounds', () => {
    expect(pairFormProblem(form)).toBeNull()
    expect(pairFormProblem({ ...form, rpoSeconds: '59' })).toMatch(/RPO/)
    expect(pairFormProblem({ ...form, rpoSeconds: '604801' })).toMatch(/RPO/)
    expect(pairFormProblem({ ...form, rpoSeconds: '60' })).toBeNull()
    expect(pairFormProblem({ ...form, name: '' })).toMatch(/required/)
    expect(pairFormProblem({ ...form, vm: 'Web_1' })).toMatch(/DNS label/)
    expect(pairFormProblem({ ...form, rtoSeconds: '-5' })).toMatch(/RTO/)
  })
  it('only allows a confirmed failover with an allowed plan and the exact phrase', () => {
    const plan = { allowed: true, blockers: [] as string[], confirm_phrase: 'failover web' }
    expect(canConfirmFailover(plan, 'failover web')).toBe(true)
    expect(canConfirmFailover(plan, 'failover web ')).toBe(false)
    expect(canConfirmFailover(plan, '')).toBe(false)
    expect(canConfirmFailover({ ...plan, allowed: false }, 'failover web')).toBe(false)
    expect(canConfirmFailover({ ...plan, blockers: ['vm running'] }, 'failover web')).toBe(false)
    expect(canConfirmFailover(null, 'failover web')).toBe(false)
  })
})

describe('volume encryption helpers', () => {
  const f: EncryptForm = { namespace: 'vms', vm: 'db', volume: 'data', size: '50Gi', storageClass: 'ceph-rbd-enc', acknowledge: false }
  it('validates the form', () => {
    expect(encryptFormProblem(f)).toBeNull()
    expect(encryptFormProblem({ ...f, vm: '' })).toMatch(/VM is required/)
    expect(encryptFormProblem({ ...f, size: 'big' })).toMatch(/Size/)
    expect(encryptFormProblem({ ...f, storageClass: '' })).toMatch(/StorageClass/)
    expect(rotateFormProblem({ namespace: 'vms', vm: 'db', volume: 'Data' })).toMatch(/Volume/)
  })
  it('builds the API request with storage_class mode', () => {
    expect(toEncryptRequest({ ...f, acknowledge: true })).toEqual({
      namespace: 'vms',
      vm: 'db',
      volume: 'data',
      size: '50Gi',
      storage_class: 'ceph-rbd-enc',
      mode: 'storage_class',
      acknowledge_unverified_encryption: true,
    })
  })
  it('describes rotation outcomes', () => {
    expect(rotateSummary({ version: 3, rewrapped: true, provider: 'local' })).toMatch(/version 3/)
    expect(rotateSummary({ version: 2, rewrapped: false, provider: 'local' })).toMatch(/nothing changed/)
  })
})
