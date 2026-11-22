// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { Link } from 'react-router'
import { ArrowRight } from 'lucide-react'

const STEPS = [
  {
    title: 'Create your first VM',
    description: 'Linux with cloud-init or Windows with VNC/RDP expose.',
    to: '/app/create',
    cta: 'Create VM',
  },
  {
    title: 'Browse virtual machines',
    description: 'List and power-manage VMs registered with Zorvia.',
    to: '/app/vms',
    cta: 'Open VMs',
  },
  {
    title: 'Review snapshots',
    description: 'Create and list snapshots for a VM.',
    to: '/app/snapshots',
    cta: 'Open snapshots',
  },
]

/** Shown when the fleet is empty — calm first-run guidance. */
export function GettingStarted() {
  return (
    <div className="zf-panel p-10 animate-fade-in">
      <h2 className="text-[28px] font-semibold tracking-[-0.03em] text-[var(--zf-ink)] mb-2">
        Get started
      </h2>
      <p className="text-[15px] text-[var(--zf-muted)] mb-10 max-w-xl">
        Your private cloud console is ready. Create a VM or explore the surfaces Zorvia supports today.
      </p>
      <div className="grid sm:grid-cols-2 gap-6">
        {STEPS.map((step) => (
          <div key={step.to} className="border-t border-[var(--zf-hairline)] pt-5">
            <h3 className="text-[17px] font-semibold tracking-[-0.02em] mb-1">{step.title}</h3>
            <p className="text-[14px] text-[var(--zf-muted)] mb-4">{step.description}</p>
            <Link to={step.to} className="inline-flex items-center gap-1 text-[14px] font-medium text-[var(--zf-link)]">
              {step.cta}
              <ArrowRight className="w-3.5 h-3.5" />
            </Link>
          </div>
        ))}
      </div>
    </div>
  )
}
