// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

import { Link } from 'react-router'
import { WHATS_NEW } from '../config/whatsNew'
import { PageHeader } from '../components/ui'

export default function Changelog() {
  return (
    <div className="space-y-6">
      <PageHeader title="What's new" description="Recent changes to Zorvia." />
      <ol className="space-y-6">
        {WHATS_NEW.map((entry) => (
          <li key={entry.id} className="zf-panel p-5">
            <div className="text-xs text-[var(--zf-muted)]">{entry.date}</div>
            <h2 className="text-[17px] font-semibold tracking-[-0.02em] text-[var(--zf-ink)]">{entry.title}</h2>
            <p className="text-sm text-[var(--zf-muted)] leading-relaxed mt-1">{entry.body}</p>
            {entry.to && (
              <Link to={entry.to} className="text-sm text-[var(--zf-cta)] mt-2 inline-block">
                Open
              </Link>
            )}
          </li>
        ))}
      </ol>
    </div>
  )
}
