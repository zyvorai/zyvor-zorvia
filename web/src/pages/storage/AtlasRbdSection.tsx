// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { useEffect, useState } from 'react'
import { HardDrive, Plus, Trash2 } from 'lucide-react'
import {
  listAtlasRbdImages,
  createAtlasRbdImage,
  deleteAtlasRbdImage,
  resizeAtlasRbdImage,
  pollAtlasJob,
  isAtlasJobTerminal,
} from '../../api/atlas'
import { useToastContext } from '../../contexts/ToastContext'
import { toastFailure } from '../../utils/toastError'
import { useConfirm } from '../../hooks/useConfirm'
import ConfirmDialog from '../../components/ConfirmDialog'

/** RBD images are a separate identity space (`rbd:<pool>/<image>`) from the
    `StorageVolume` abstraction the main Atlas section manages -- Atlas's own
    list route returns just names, not full objects, so this section is
    deliberately lighter-weight than the volumes list. Real in `real` Ceph
    driver mode, pure DB bookkeeping in the `fake` mode used for local dev. */
export default function AtlasRbdSection() {
  const toast = useToastContext()
  const { confirmState, confirm, cancel } = useConfirm()
  const [pool, setPool] = useState('rbd')
  const [images, setImages] = useState<string[]>([])
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState<string | null>(null)

  const load = async () => {
    try {
      const res = await listAtlasRbdImages(pool)
      setImages(res.images)
    } catch (e) {
      toastFailure(toast, 'Failed to list RBD images', e)
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    void load()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pool])

  const runWrite = async (busyKey: string, actionLabel: string, name: string, op: () => Promise<{ job_id: string }>) => {
    setBusy(busyKey)
    try {
      const envelope = await op()
      toast.success(`${actionLabel} requested for '${name}'`)
      const job = await pollAtlasJob(envelope.job_id)
      if (isAtlasJobTerminal(job)) {
        if (job.state === 'succeeded') {
          toast.success(`${actionLabel} succeeded for '${name}'`)
        } else {
          toast.error(`${actionLabel} failed for '${name}': ${job.error ?? 'unknown error'}`)
        }
      }
      await load()
    } catch (e) {
      toastFailure(toast, `Failed to ${actionLabel.toLowerCase()}`, e)
    } finally {
      setBusy(null)
    }
  }

  return (
    <div className="bg-[var(--zf-surface)] rounded-lg border border-[var(--zf-hairline)] p-6 space-y-4">
      <div className="flex items-center justify-between flex-wrap gap-3">
        <h2 className="flex items-center gap-2 text-sm font-semibold text-[var(--zf-ink)]">
          <HardDrive className="w-4 h-4 text-[var(--zf-muted)]" />
          RBD images
        </h2>
        <div className="flex items-center gap-2">
          <label className="text-xs text-[var(--zf-muted)]">Pool</label>
          <input
            value={pool}
            onChange={(e) => setPool(e.target.value)}
            className="w-28 px-2 py-1 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-xs"
          />
        </div>
      </div>

      {!loading && images.length === 0 && (
        <p className="text-sm text-[var(--zf-muted)]">No RBD images in pool '{pool}'.</p>
      )}

      {images.length > 0 && (
        <div className="divide-y divide-[var(--zf-hairline)]">
          {images.map((image) => (
            <RbdImageRow
              key={image}
              pool={pool}
              image={image}
              busy={busy}
              onResize={(sizeBytes) => void runWrite(`resize-${image}`, 'Resize', image, () => resizeAtlasRbdImage(pool, image, sizeBytes))}
              onDelete={async () => {
                if (
                  !(await confirm(`Delete RBD image '${image}'`, `Delete rbd:${pool}/${image}? Any data on it is destroyed.`, {
                    variant: 'danger',
                    confirmLabel: 'Delete',
                  }))
                ) {
                  return
                }
                void runWrite(`delete-${image}`, 'Delete', image, () => deleteAtlasRbdImage(pool, image))
              }}
            />
          ))}
        </div>
      )}

      <CreateRbdImageForm
        pool={pool}
        disabled={busy !== null}
        onCreate={(name, sizeGiB) =>
          void runWrite('create-rbd', 'Create', name, () =>
            createAtlasRbdImage({ name, pool, size_bytes: sizeGiB * 1024 * 1024 * 1024 }),
          )
        }
      />

      {confirmState && (
        <ConfirmDialog
          title={confirmState.title}
          message={confirmState.message}
          confirmLabel={confirmState.confirmLabel}
          variant={confirmState.variant}
          onConfirm={confirmState.onConfirm}
          onCancel={cancel}
        />
      )}
    </div>
  )
}

function RbdImageRow({
  pool,
  image,
  busy,
  onResize,
  onDelete,
}: {
  pool: string
  image: string
  busy: string | null
  onResize: (sizeBytes: number) => void
  onDelete: () => void
}) {
  const [newSizeGiB, setNewSizeGiB] = useState(20)
  return (
    <div className="flex items-center justify-between py-2 gap-3">
      <div className="font-medium text-sm text-[var(--zf-ink)]">
        rbd:{pool}/{image}
      </div>
      <div className="flex items-center gap-2 shrink-0">
        <input
          type="number"
          min={1}
          value={newSizeGiB}
          onChange={(e) => setNewSizeGiB(parseInt(e.target.value) || 1)}
          className="w-20 px-2 py-1 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-xs"
          title="New size (GiB)"
        />
        <button
          type="button"
          disabled={busy !== null}
          onClick={() => onResize(newSizeGiB * 1024 * 1024 * 1024)}
          className="zf-btn zf-btn-ghost zf-btn-sm"
        >
          Resize
        </button>
        <button type="button" disabled={busy !== null} onClick={onDelete} className="zf-btn zf-btn-danger zf-btn-sm">
          <Trash2 className="w-3.5 h-3.5" />
        </button>
      </div>
    </div>
  )
}

function CreateRbdImageForm({
  pool,
  disabled,
  onCreate,
}: {
  pool: string
  disabled: boolean
  onCreate: (name: string, sizeGiB: number) => void
}) {
  const [name, setName] = useState('')
  const [sizeGiB, setSizeGiB] = useState(10)

  return (
    <div className="flex items-end gap-2 flex-wrap pt-2 border-t border-[var(--zf-hairline)]">
      <div>
        <label className="block text-xs text-[var(--zf-muted)] mb-1">Image name</label>
        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="app-image"
          className="w-40 px-2.5 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm"
        />
      </div>
      <div>
        <label className="block text-xs text-[var(--zf-muted)] mb-1">Size (GiB)</label>
        <input
          type="number"
          min={1}
          value={sizeGiB}
          onChange={(e) => setSizeGiB(parseInt(e.target.value) || 1)}
          className="w-24 px-2.5 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm"
        />
      </div>
      <button
        type="button"
        disabled={disabled || !name.trim() || sizeGiB < 1}
        onClick={() => {
          onCreate(name.trim(), sizeGiB)
          setName('')
        }}
        className="zf-btn zf-btn-primary zf-btn-sm"
      >
        <Plus className="w-4 h-4" />
        Create image in '{pool}'
      </button>
    </div>
  )
}
