// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { Link } from 'react-router'
import MarketingLayout from '../../components/MarketingLayout'
import {
  ConsoleStage,
  ExposeStage,
  MktBand,
  MktChapter,
  MktHero,
  TerminalStage,
} from '../../components/marketing/MktSections'

const SURFACES = [
  {
    id: 'web',
    title: 'Web.',
    lede: 'Apple-calm console for create, power, console, expose, and snapshots.',
    visual: <ConsoleStage eyebrow="Web" headline="/app · signed in" />,
    inverse: false,
  },
  {
    id: 'cli',
    title: 'CLI.',
    lede: 'zorvia — templates, profiles, blueprints, and scriptable JSON or YAML.',
    visual: <TerminalStage />,
    inverse: true,
  },
  {
    id: 'operator',
    title: 'Operator.',
    lede: 'KubeVirt VirtualMachine CRDs on the cluster — reconciled and operable from Zorvia.',
    visual: <ConsoleStage eyebrow="Kubernetes" headline="VirtualMachine · VMI" />,
    inverse: false,
  },
  {
    id: 'api',
    title: 'API.',
    lede: 'REST and WebSocket control plane for automation — same auth as the console.',
    visual: <ExposeStage />,
    inverse: true,
  },
]

export default function Platform() {
  return (
    <MarketingLayout>
      <MktHero
        compact
        kicker="Zorvia"
        title={
          <>
            Four ways in.
            <br />
            One control plane.
          </>
        }
        punch="Same APIs underneath."
        lede="Web, CLI, operator, and HTTP — every interface talks to Zorvia over one surface."
      />

      {SURFACES.map((s) => (
        <MktChapter key={s.id} id={s.id} title={s.title} lede={s.lede} inverse={s.inverse}>
          {s.visual}
        </MktChapter>
      ))}

      <MktBand
        title="Start with the console."
        lede="Or stay in the terminal — both land on the same cluster."
        cta={
          <Link to="/sign-in" className="zf-btn mkt-band-cta">
            Sign in
          </Link>
        }
      />
    </MarketingLayout>
  )
}
