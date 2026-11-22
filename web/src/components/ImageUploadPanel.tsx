// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { useEffect, useRef, useState } from 'react'
import { Upload, Loader2, CheckCircle2, X } from 'lucide-react'
import { Card, CardBody } from './ui'
import ExperimentalDisabledNotice, { ExperimentalBadge, INPUT_CLASS } from './ExperimentalDisabledNotice'
import { getUploadConfig, probeImageUploadDisabled, uploadImage, validateImage, type UploadResult, type ValidateResult } from '../api/imageUpload'
import { IMAGE_FORMATS, formatCap, formatPercent, uploadFormProblem } from '../utils/imageUpload'
import { isExperimentalDisabled } from '../utils/experimental'
import { formatBytes } from '../utils/format'
import { formatUserError } from '../utils/apiError'
import { useToastContext } from '../contexts/ToastContext'
import { usePermissions } from '../hooks/usePermissions'

/** Upload a disk image into CDI (Experimental, docs/IMAGE_UPLOAD.md). Needs
    write access; viewers see a read-only note instead of the form. */
export default function ImageUploadPanel({ onUploaded }: { onUploaded?: () => void }) {
  const toast = useToastContext()
  const { canWrite } = usePermissions()
  const [disabled, setDisabled] = useState<string | null>(null)
  const [file, setFile] = useState<File | null>(null)
  const [name, setName] = useState('')
  const [size, setSize] = useState('20Gi')
  const [namespace, setNamespace] = useState('')
  const [storageClass, setStorageClass] = useState('')
  const [format, setFormat] = useState('')
  const [sha256, setSha256] = useState('')
  const [validation, setValidation] = useState<ValidateResult | null>(null)
  const [progress, setProgress] = useState<{ loaded: number; total: number } | null>(null)
  const [busy, setBusy] = useState<'validate' | 'upload' | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [result, setResult] = useState<UploadResult | null>(null)
  const [maxBytes, setMaxBytes] = useState<number | null>(null)
  const abortRef = useRef<AbortController | null>(null)

  useEffect(() => {
    if (!canWrite) return
    let alive = true
    probeImageUploadDisabled().then((m) => alive && setDisabled(m))
    getUploadConfig().then((c) => alive && setMaxBytes(c ? c.max_bytes : null))
    return () => {
      alive = false
    }
  }, [canWrite])

  if (disabled !== null) {
    return <ExperimentalDisabledNotice feature="Disk image upload" message={disabled} />
  }

  if (!canWrite) {
    return (
      <p className="text-xs text-[var(--zf-muted)]" data-testid="upload-readonly">
        Uploading disk images needs the user or admin role.
      </p>
    )
  }

  const problem = uploadFormProblem({ name, size, sha256, hasFile: !!file })

  const handleError = (e: unknown) => {
    if (isExperimentalDisabled(e)) setDisabled(e.message)
    else setError(formatUserError(e))
  }

  const onValidate = async () => {
    if (!file) return
    setBusy('validate')
    setError(null)
    setValidation(null)
    try {
      setValidation(await validateImage(file, format || undefined))
    } catch (e) {
      handleError(e)
    } finally {
      setBusy(null)
    }
  }

  const onUpload = async () => {
    if (!file || problem) return
    setBusy('upload')
    setError(null)
    setResult(null)
    setProgress({ loaded: 0, total: file.size })
    const ctl = new AbortController()
    abortRef.current = ctl
    try {
      const r = await uploadImage(name, file, {
        size,
        namespace: namespace || undefined,
        storageClass: storageClass || undefined,
        format: format || undefined,
        sha256: sha256 || undefined,
        onProgress: (loaded, total) => setProgress({ loaded, total }),
        signal: ctl.signal,
        maxBytes,
      })
      setResult(r)
      toast.success(`Uploaded ${r.name} (${formatBytes(r.bytes)}, ${r.format})`)
      onUploaded?.()
    } catch (e) {
      handleError(e)
    } finally {
      abortRef.current = null
      setBusy(null)
    }
  }

  const pct = progress ? formatPercent(progress.loaded, progress.total) : 0

  return (
    <Card>
      <CardBody className="space-y-4">
        <div className="flex items-center gap-2 text-sm font-medium text-[var(--zf-ink)]">
          <Upload className="w-4 h-4" aria-hidden="true" />
          Upload a disk image
          <ExperimentalBadge />
        </div>
        <p className="text-xs text-[var(--zf-muted)]">
          Creates a CDI DataVolume and streams the file through the upload proxy. The server enforces a size cap (
          <span data-testid="upload-cap">{maxBytes ? formatCap(maxBytes) : 'default 20 GiB'}</span>) and aborts the upload if it is exceeded. qcow2, vmdk, vhdx, vhd and vdi are converted to raw by CDI.
          PVC size must cover the virtual disk size, not just the file size.
        </p>
        <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
          <input
            type="file"
            aria-label="Disk image file"
            disabled={busy !== null}
            onChange={(e) => {
              setFile(e.target.files?.[0] ?? null)
              setValidation(null)
              setResult(null)
              setError(null)
              setProgress(null)
            }}
            className={`md:col-span-3 ${INPUT_CLASS}`}
          />
          <input aria-label="DataVolume name" placeholder="Name (e.g. ubuntu-24)" value={name} onChange={(e) => setName(e.target.value)} disabled={busy !== null} className={INPUT_CLASS} />
          <input aria-label="PVC size" placeholder="PVC size (20Gi)" value={size} onChange={(e) => setSize(e.target.value)} disabled={busy !== null} className={INPUT_CLASS} />
          <select aria-label="Declared format" value={format} onChange={(e) => setFormat(e.target.value)} disabled={busy !== null} className={INPUT_CLASS}>
            <option value="">Detect format</option>
            {IMAGE_FORMATS.map((f) => (
              <option key={f} value={f}>{f}{f === 'raw' ? ' (must be declared)' : ''}</option>
            ))}
          </select>
          <input aria-label="Namespace" placeholder="Namespace (optional)" value={namespace} onChange={(e) => setNamespace(e.target.value)} disabled={busy !== null} className={INPUT_CLASS} />
          <input aria-label="StorageClass" placeholder="StorageClass (optional)" value={storageClass} onChange={(e) => setStorageClass(e.target.value)} disabled={busy !== null} className={INPUT_CLASS} />
          <input aria-label="Expected SHA-256" placeholder="SHA-256 (optional)" value={sha256} onChange={(e) => setSha256(e.target.value.trim())} disabled={busy !== null} className={INPUT_CLASS} />
        </div>

        {file && (
          <p className="text-xs text-[var(--zf-muted)]">
            {file.name}: {formatBytes(file.size)}
          </p>
        )}
        {file && problem && <p className="text-xs text-[var(--zf-warning)]" role="alert">{problem}</p>}

        <div className="flex flex-wrap items-center gap-2">
          <button type="button" className="zf-btn zf-btn-ghost zf-btn-sm" disabled={!file || busy !== null} onClick={onValidate}>
            {busy === 'validate' && <Loader2 className="w-3.5 h-3.5 animate-spin" />}
            Validate only
          </button>
          <button type="button" className="zf-btn zf-btn-primary zf-btn-sm" disabled={!file || !!problem || busy !== null} onClick={onUpload}>
            {busy === 'upload' && <Loader2 className="w-3.5 h-3.5 animate-spin" />}
            Upload
          </button>
          {busy === 'upload' && (
            <button type="button" className="zf-btn zf-btn-ghost zf-btn-sm" onClick={() => abortRef.current?.abort()}>
              <X className="w-3.5 h-3.5" /> Cancel
            </button>
          )}
        </div>

        {progress && (
          <div>
            <div
              role="progressbar"
              aria-label="Upload progress"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={pct}
              className="h-2 rounded bg-[var(--zf-fill-tertiary)] overflow-hidden"
            >
              <div className="h-full bg-[var(--zf-cta)]" style={{ width: `${pct}%` }} />
            </div>
            <p className="text-xs text-[var(--zf-muted)] mt-1">
              {pct}% sent ({formatBytes(progress.loaded)} of {formatBytes(progress.total)})
              {busy === 'upload' && pct >= 100 ? ': waiting for CDI to finish' : ''}
            </p>
          </div>
        )}

        {validation && (
          <p className={`text-sm ${validation.valid ? 'text-[var(--zf-ink)]' : 'text-[var(--zf-danger)]'}`} role="status">
            {validation.valid
              ? `Valid ${validation.format} image${validation.converted_to_raw_by_cdi ? ' (CDI will convert it to raw)' : ''}.`
              : `Not accepted: ${validation.error ?? 'unknown format'}`}
          </p>
        )}
        {result && (
          <p className="text-sm text-[var(--zf-ink)] flex items-center gap-1.5" role="status">
            <CheckCircle2 className="w-4 h-4 text-[var(--zf-success)]" aria-hidden="true" />
            Uploaded {result.name}: {formatBytes(result.bytes)}, {result.format}, sha256 {result.sha256.slice(0, 12)}…
          </p>
        )}
        {error && <p className="text-sm text-[var(--zf-danger)]" role="alert">{error}</p>}
      </CardBody>
    </Card>
  )
}
