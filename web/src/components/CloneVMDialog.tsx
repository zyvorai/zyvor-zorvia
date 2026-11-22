// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { useState } from 'react'
import { Copy } from 'lucide-react'
import { cloneVM } from '../api/vm'
import { useToastContext } from '../contexts/ToastContext'
import { toastFailure } from '../utils/toastError'

interface CloneVMDialogProps {
  vmName: string
  onClose: () => void
  onSuccess: () => void
}

export default function CloneVMDialog({ vmName, onClose, onSuccess }: CloneVMDialogProps) {
  const toast = useToastContext()
  const [targetName, setTargetName] = useState(`${vmName}-clone`)
  const [includeSnapshots, setIncludeSnapshots] = useState(false)
  const [linkedClone, setLinkedClone] = useState(false)
  const [isCloning, setIsCloning] = useState(false)

  const handleClone = async () => {
    if (!targetName.trim()) {
      toast.error('Please enter a name for the cloned VM')
      return
    }

    setIsCloning(true)
    try {
      await cloneVM(vmName, targetName, {
        includeSnapshots,
        linkedClone,
      })
      toast.success(`VM '${vmName}' cloned to '${targetName}' successfully`)
      onSuccess()
      onClose()
    } catch (error) {
      toastFailure(toast, 'Failed to clone VM', error)
    } finally {
      setIsCloning(false)
    }
  }

  return (
    <div className="bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-xl p-5 space-y-4">
      <h3 className="text-sm font-semibold text-[var(--zf-ink)]">Clone VM</h3>

      <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
        <div>
          <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Source VM</label>
          <input
            type="text"
            value={vmName}
            disabled
            className="input-field text-sm text-[var(--zf-muted)]"
          />
        </div>

        <div>
          <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">New VM Name</label>
          <input
            type="text"
            value={targetName}
            onChange={(e) => setTargetName(e.target.value)}
            placeholder="Enter name for cloned VM"
            className="input-field text-sm"
          />
        </div>
      </div>

      <div className="space-y-2">
        <div className="flex items-center gap-2">
          <input
            type="checkbox"
            id="includeSnapshots"
            checked={includeSnapshots}
            onChange={(e) => setIncludeSnapshots(e.target.checked)}
            className="w-4 h-4 text-[var(--zf-link)] bg-[var(--zf-surface)] border-[var(--zf-hairline)] rounded focus:ring-[var(--zf-link)]"
          />
          <label htmlFor="includeSnapshots" className="text-sm text-[var(--zf-ink)]">
            Include snapshots
          </label>
        </div>

        <div className="flex items-center gap-2">
          <input
            type="checkbox"
            id="linkedClone"
            checked={linkedClone}
            onChange={(e) => setLinkedClone(e.target.checked)}
            className="w-4 h-4 text-[var(--zf-link)] bg-[var(--zf-surface)] border-[var(--zf-hairline)] rounded focus:ring-[var(--zf-link)]"
          />
          <label htmlFor="linkedClone" className="text-sm text-[var(--zf-ink)]">
            Linked clone (faster, uses less space)
          </label>
        </div>
      </div>

      <p className="text-sm text-[var(--zf-muted)]">
        {linkedClone
          ? "Linked clone creates a VM that shares disk with the source. Changes won't affect the source."
          : 'Full clone creates an independent copy of the VM. This may take some time.'}
      </p>

      <div className="flex gap-2">
        <button
          onClick={() => void handleClone()}
          disabled={isCloning}
          className="zf-btn zf-btn-primary zf-btn-sm"
        >
          {isCloning ? (
            <>
              <div className="animate-spin rounded-full h-4 w-4 border-b-2 border-white"></div>
              Cloning...
            </>
          ) : (
            <>
              <Copy className="w-4 h-4" />
              Clone VM
            </>
          )}
        </button>
        <button
          onClick={onClose}
          disabled={isCloning}
          className="zf-btn zf-btn-ghost zf-btn-sm"
        >
          Cancel
        </button>
      </div>
    </div>
  )
}
