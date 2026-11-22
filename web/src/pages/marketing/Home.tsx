// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { Link } from 'react-router'
import { useAuth } from '../../contexts/AuthContext'
import MarketingLayout from '../../components/MarketingLayout'
import {
  AuthCtas,
  ConsoleStage,
  ExposeStage,
  MktBand,
  MktChapter,
  MktHero,
  MktHighlights,
  TerminalStage,
  VncStage,
} from '../../components/marketing/MktSections'

export default function Home() {
  const { isAuthenticated } = useAuth()

  return (
    <MarketingLayout>
      <MktHero
        kicker="Zorvia"
        title="Private cloud."
        punch="Beautifully simple."
        lede="One control plane for Linux VMs — create, console, expose, and day-2 ops without the heavyweight stack."
        cta={<AuthCtas isAuthenticated={isAuthenticated} />}
        visual={<ConsoleStage />}
      />

      <div className="apple-metric-band apple-metric-band--flat mkt-reveal" aria-label="What you get" style={{ maxWidth: '100%', margin: '0 auto 48px' }}>
        <div>
          <b>Create</b>
          <span>Linux & Windows</span>
        </div>
        <div>
          <b>Console</b>
          <span>Serial · VNC</span>
        </div>
        <div>
          <b>Expose</b>
          <span>SSH · RDP</span>
        </div>
        <div>
          <b>Day-2</b>
          <span>Snapshots · power</span>
        </div>
      </div>

      <MktHighlights
        items={[
          {
            id: 'create',
            label: 'Create VM',
            title: 'Create in minutes.',
            body: 'Linux with cloud-init, or Windows with VNC and RDP expose — blank disks, PVCs, or containerdisk images.',
            visual: <ConsoleStage eyebrow="Create" headline="Linux · Windows · cloud-init" />,
          },
          {
            id: 'console',
            label: 'Serial + VNC',
            title: 'Console when it matters.',
            body: 'Authenticated WebSocket proxies to KubeVirt serial and VNC — from the browser, when the VMI is Running.',
            visual: <TerminalStage />,
          },
          {
            id: 'expose',
            label: 'Expose',
            title: 'Reach the guest.',
            body: 'NodePort Services for SSH, VNC, and RDP — labeled per VM, with the host you configure for operators.',
            visual: <ExposeStage />,
          },
          {
            id: 'snapshots',
            label: 'Snapshots',
            title: 'Capture. Restore.',
            body: 'Create, list, delete, and revert snapshots from the console — same cluster, same APIs as the CLI.',
            visual: <VncStage />,
          },
        ]}
      />

      <MktChapter
        title="Designed for operators."
        lede="A calm console for fleet power, networking, and policy — backed by a single Rust daemon on KubeVirt."
      >
        <ConsoleStage eyebrow="Day-2" headline="Power. Console. Expose." />
      </MktChapter>

      <MktBand
        title="CLI. Web. One API."
        lede="Every surface talks to the same control plane — script it, click it, or automate it."
        cta={
          <Link to="/platform" className="zf-btn mkt-band-cta">
            See the platform
          </Link>
        }
      />
    </MarketingLayout>
  )
}
