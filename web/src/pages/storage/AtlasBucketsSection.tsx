// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { useEffect, useState } from 'react'
import { Archive, Plus, Trash2 } from 'lucide-react'
import {
  listAtlasBuckets,
  createAtlasBucket,
  deleteAtlasBucket,
  pollAtlasJob,
  isAtlasJobTerminal,
  AtlasBucket,
} from '../../api/atlas'
import { useToastContext } from '../../contexts/ToastContext'
import { toastFailure } from '../../utils/toastError'
import { useConfirm } from '../../hooks/useConfirm'
import ConfirmDialog from '../../components/ConfirmDialog'

/** Object-store buckets (RGW/S3, provisioned via an ObjectBucketClaim) --
    another Atlas resource type distinct from volumes/RBD images. Create and
    delete are async jobs on Atlas's side, same as volumes. Object-level S3
    operations (list/upload/download/prune) and backup/restore-creation UI
    are deliberately out of scope for this pass -- this section covers
    bucket lifecycle only. */
export default function AtlasBucketsSection() {
  const toast = useToastContext()
  const { confirmState, confirm, cancel } = useConfirm()
  const [buckets, setBuckets] = useState<AtlasBucket[]>([])
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState<string | null>(null)

  const load = async () => {
    try {
      setBuckets(await listAtlasBuckets())
    } catch (e) {
      toastFailure(toast, 'Failed to list buckets', e)
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    void load()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

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
      <h2 className="flex items-center gap-2 text-sm font-semibold text-[var(--zf-ink)]">
        <Archive className="w-4 h-4 text-[var(--zf-muted)]" />
        Object-store buckets
      </h2>

      {!loading && buckets.length === 0 && <p className="text-sm text-[var(--zf-muted)]">No buckets yet.</p>}

      {buckets.length > 0 && (
        <div className="divide-y divide-[var(--zf-hairline)]">
          {buckets.map((b) => (
            <BucketRow
              key={b.id}
              bucket={b}
              busy={busy}
              onDelete={async () => {
                if (
                  !(await confirm(`Delete bucket '${b.name}'`, 'Delete this bucket? Objects in it are lost unless forced/empty.', {
                    variant: 'danger',
                    confirmLabel: 'Delete',
                  }))
                ) {
                  return
                }
                void runWrite(`delete-${b.id}`, 'Delete', b.name, () => deleteAtlasBucket(b.id, false))
              }}
            />
          ))}
        </div>
      )}

      <CreateBucketForm
        disabled={busy !== null}
        onCreate={(name) => void runWrite('create-bucket', 'Create', name, () => createAtlasBucket({ name }))}
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

function BucketRow({ bucket, busy, onDelete }: { bucket: AtlasBucket; busy: string | null; onDelete: () => void }) {
  return (
    <div className="flex items-center justify-between py-2 gap-3">
      <div>
        <div className="font-medium text-sm text-[var(--zf-ink)]">{bucket.name}</div>
        <div className="text-xs text-[var(--zf-muted)]">
          {bucket.bucket_name ?? bucket.name}
          {bucket.namespace ? ` · ${bucket.namespace}` : ''}
        </div>
      </div>
      <div className="flex items-center gap-2 shrink-0">
        <span className="px-2 py-0.5 rounded-full text-xs font-medium border text-[var(--zf-muted)] bg-[var(--zf-canvas)] border-[var(--zf-hairline)]">
          {bucket.state}
        </span>
        <button type="button" disabled={busy !== null} onClick={onDelete} className="zf-btn zf-btn-danger zf-btn-sm">
          <Trash2 className="w-3.5 h-3.5" />
        </button>
      </div>
    </div>
  )
}

function CreateBucketForm({ disabled, onCreate }: { disabled: boolean; onCreate: (name: string) => void }) {
  const [name, setName] = useState('')
  return (
    <div className="flex items-end gap-2 flex-wrap pt-2 border-t border-[var(--zf-hairline)]">
      <div>
        <label className="block text-xs text-[var(--zf-muted)] mb-1">Bucket name</label>
        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="app-uploads"
          className="w-48 px-2.5 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm"
        />
      </div>
      <button
        type="button"
        disabled={disabled || !name.trim()}
        onClick={() => {
          onCreate(name.trim())
          setName('')
        }}
        className="zf-btn zf-btn-primary zf-btn-sm"
      >
        <Plus className="w-4 h-4" />
        Create bucket
      </button>
    </div>
  )
}
