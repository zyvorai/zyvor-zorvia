// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

import { useEffect, useState } from 'react'
import { Link } from 'react-router'
import { Sparkles, X } from 'lucide-react'
import { WHATS_NEW } from '../config/whatsNew'
import { latestId, readLastSeen, unseenCount, writeLastSeen } from '../utils/whatsNew'

export default function WhatsNew() {
  const [open, setOpen] = useState(false)
  const [lastSeen, setLastSeen] = useState(readLastSeen)
  const unseen = unseenCount(WHATS_NEW, lastSeen)

  const close = () => {
    const id = latestId(WHATS_NEW)
    writeLastSeen(id)
    setLastSeen(id)
    setOpen(false)
  }

  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [open])

  return (
    <>
      <button
        type="button"
        className="console-icon-btn relative"
        onClick={() => setOpen(true)}
        title="What's new"
        aria-label={unseen > 0 ? `What's new, ${unseen} unread` : "What's new"}
      >
        <Sparkles className="w-3.5 h-3.5" />
        {unseen > 0 && (
          <span
            className="absolute -top-0.5 -right-0.5 w-2 h-2 rounded-full bg-[var(--zf-cta)]"
            aria-hidden="true"
          />
        )}
      </button>
      {open && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm animate-fade-in"
          role="dialog"
          aria-modal="true"
          aria-labelledby="whats-new-title"
          onClick={close}
        >
          <div
            className="bg-[var(--zf-canvas)] rounded-xl shadow-2xl border border-[var(--zf-hairline)] w-full max-w-md p-6 max-h-[80vh] overflow-y-auto"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex items-center justify-between mb-4">
              <h3 id="whats-new-title" className="text-base font-semibold text-[var(--zf-ink)]">
                What's new
              </h3>
              <button type="button" className="console-icon-btn" onClick={close} aria-label="Close">
                <X className="w-3.5 h-3.5" />
              </button>
            </div>
            <ul className="space-y-4">
              {WHATS_NEW.map((e) => (
                <li key={e.id}>
                  <div className="text-xs text-[var(--zf-muted)]">
                    {e.date}
                    {e.id > lastSeen && <span className="ml-2 text-[var(--zf-cta)]">New</span>}
                  </div>
                  <div className="text-sm font-medium text-[var(--zf-ink)]">{e.title}</div>
                  <p className="text-sm text-[var(--zf-muted)] leading-relaxed">{e.body}</p>
                  {e.to && (
                    <Link to={e.to} onClick={close} className="text-xs text-[var(--zf-cta)]">
                      Open
                    </Link>
                  )}
                </li>
              ))}
            </ul>
          </div>
        </div>
      )}
    </>
  )
}
