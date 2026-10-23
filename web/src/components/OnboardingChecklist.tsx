// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

import { useState } from 'react'
import { Link } from 'react-router'
import { Check, X } from 'lucide-react'
import {
  ONBOARDING_STEPS,
  onboardingProgress,
  readChecked,
  readDismissed,
  writeChecked,
  writeDismissed,
} from '../utils/onboarding'

interface OnboardingChecklistProps {
  /** A VM already exists, so the first step counts as done. */
  hasVMs: boolean
}

export default function OnboardingChecklist({ hasVMs }: OnboardingChecklistProps) {
  const [checked, setChecked] = useState(readChecked)
  const [dismissed, setDismissed] = useState(readDismissed)
  const progress = onboardingProgress(ONBOARDING_STEPS, checked, hasVMs)

  if (dismissed || progress.complete) return null

  const toggle = (id: string) => {
    const next = new Set(checked)
    if (next.has(id)) next.delete(id)
    else next.add(id)
    writeChecked(next)
    setChecked(next)
  }

  const dismiss = () => {
    writeDismissed()
    setDismissed(true)
  }

  return (
    <section className="zf-panel p-6" aria-labelledby="onboarding-title">
      <div className="flex items-start justify-between gap-4">
        <div>
          <h2 id="onboarding-title" className="text-[17px] font-semibold tracking-[-0.02em] text-[var(--zf-ink)]">
            Set up Zorvia
          </h2>
          <p className="text-[13px] text-[var(--zf-muted)]">
            {progress.done} of {progress.total} done
          </p>
        </div>
        <button type="button" className="console-icon-btn" onClick={dismiss} aria-label="Dismiss setup checklist">
          <X className="w-3.5 h-3.5" />
        </button>
      </div>
      <div
        className="mt-3 h-1.5 rounded-full bg-[var(--zf-hairline)]"
        role="progressbar"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={progress.pct}
        aria-label="Setup progress"
      >
        <div className="h-full rounded-full bg-[var(--zf-cta)]" style={{ width: `${progress.pct}%` }} />
      </div>
      <ul className="mt-4 divide-y divide-[var(--zf-hairline)]">
        {ONBOARDING_STEPS.map((step) => {
          const isDone = checked.has(step.id) || (step.id === 'vm' && hasVMs)
          return (
            <li key={step.id} className="flex items-center gap-3 py-2.5">
              <button
                type="button"
                role="checkbox"
                aria-checked={isDone}
                aria-label={`Mark "${step.title}" as ${isDone ? 'not done' : 'done'}`}
                onClick={() => toggle(step.id)}
                disabled={step.id === 'vm' && hasVMs}
                className={`w-5 h-5 shrink-0 rounded-full border flex items-center justify-center ${
                  isDone ? 'bg-[var(--zf-cta)] border-[var(--zf-cta)] text-white' : 'border-[var(--zf-hairline)]'
                }`}
              >
                {isDone && <Check className="w-3 h-3" />}
              </button>
              <div className="min-w-0 flex-1">
                <div className={`text-[14px] ${isDone ? 'line-through text-[var(--zf-muted)]' : 'text-[var(--zf-ink)]'}`}>
                  {step.title}
                </div>
                <div className="text-[12px] text-[var(--zf-muted)]">{step.description}</div>
              </div>
              <Link to={step.to} className="text-[13px] font-medium text-[var(--zf-link)] shrink-0">
                {step.cta}
              </Link>
            </li>
          )
        })}
      </ul>
    </section>
  )
}
