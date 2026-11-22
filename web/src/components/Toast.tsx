// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { useEffect } from 'react'
import { CheckCircle, XCircle, AlertCircle, Info, X } from 'lucide-react'

export type ToastType = 'success' | 'error' | 'warning' | 'info'

export interface Toast {
  id: string
  type: ToastType
  message: string
  duration?: number
}

interface ToastProps {
  toast: Toast
  onClose: (id: string) => void
}

/** `message` is already fully formatted by the caller (e.g. `toastFailure`)
 * -- don't re-run it through error formatting again here, that was masking
 * a real "no toast ever appears" bug by double-processing an already-safe
 * string. */
function displayMessage(_type: ToastType, message: string): string {
  return message.length > 320 ? `${message.slice(0, 317)}…` : message
}

export function ToastItem({ toast, onClose }: ToastProps) {
  useEffect(() => {
    const duration = toast.duration || 5000
    const timer = setTimeout(() => onClose(toast.id), duration)
    return () => clearTimeout(timer)
  }, [toast, onClose])

  const display = displayMessage(toast.type, toast.message)

  const config = {
    success: { icon: CheckCircle, bg: 'bg-[var(--zf-success)]/10', border: 'border-[var(--zf-success)]/25', text: 'text-[var(--zf-success)]' },
    error: { icon: XCircle, bg: 'bg-[var(--zf-danger)]/10', border: 'border-[var(--zf-danger)]/25', text: 'text-[var(--zf-danger)]' },
    warning: { icon: AlertCircle, bg: 'bg-[var(--zf-warning)]/10', border: 'border-[var(--zf-warning)]/25', text: 'text-[var(--zf-warning)]' },
    info: { icon: Info, bg: 'bg-[var(--zf-link)]/10', border: 'border-[var(--zf-link)]/25', text: 'text-[var(--zf-link)]' },
  }[toast.type]

  const Icon = config.icon

  return (
    <div
      className={`flex items-center gap-3 px-4 py-3 rounded-xl border shadow-lg animate-slide-in bg-[var(--zf-surface)] ${config.bg} ${config.border}`}
    >
      <Icon className={`w-4 h-4 shrink-0 ${config.text}`} />
      <span className="flex-1 text-sm text-[var(--zf-ink)] whitespace-pre-wrap break-words">{display}</span>
      <button
        onClick={() => onClose(toast.id)}
        className="shrink-0 p-0.5 rounded-md text-[var(--zf-muted)] hover:text-[var(--zf-ink)] transition-colors"
      >
        <X className="w-3.5 h-3.5" />
      </button>
    </div>
  )
}

interface ToastContainerProps {
  toasts: Toast[]
  onClose: (id: string) => void
}

export function ToastContainer({ toasts, onClose }: ToastContainerProps) {
  return (
    <div className="fixed top-4 right-4 z-50 space-y-2 max-w-sm">
      {toasts.map((toast) => (
        <ToastItem key={toast.id} toast={toast} onClose={onClose} />
      ))}
    </div>
  )
}
