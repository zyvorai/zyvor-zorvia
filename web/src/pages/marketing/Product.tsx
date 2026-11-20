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
  VncStage,
} from '../../components/marketing/MktSections'

const SUBNAV = [
  { label: 'Overview', href: '#overview' },
  { label: 'VMs', href: '#vms' },
  { label: 'Console', href: '#console' },
  { label: 'Expose', href: '#expose' },
  { label: 'Snapshots', href: '#snapshots' },
]

export default function Product() {
  return (
    <MarketingLayout subnav={SUBNAV}>
      <div id="overview">
        <MktHero
          compact
          kicker="Zorvia"
          title={
            <>
              The control plane
              <br />
              for your private cloud.
            </>
          }
          punch="Built for day-2."
          lede="Create and operate KubeVirt VMs with cloud-init, serial console, VNC, NodePort expose, clone, and snapshots — from one product."
          cta={
            <Link to="/sign-in" className="zf-btn zf-btn-primary">
              Sign in to console
            </Link>
          }
        />
      </div>

      <MktChapter
        id="vms"
        title="Virtual machines."
        lede="Lifecycle, templates, cloud-init, and bulk power actions — one model across web, CLI, and API."
      >
        <ConsoleStage eyebrow="VMs" headline="Create. Start. Stop. Clone." />
      </MktChapter>

      <MktChapter
        id="console"
        title="Serial and VNC."
        lede="Open a live console when the guest is Running — terminal for serial, graphical VNC when you need the screen."
        inverse
      >
        <TerminalStage />
      </MktChapter>

      <MktChapter
        id="expose"
        title="Expose SSH, VNC, RDP."
        lede="Publish guest ports as Kubernetes NodePort Services. Operators get a host and port — no Fabric host NAT required."
      >
        <ExposeStage />
      </MktChapter>

      <MktChapter
        id="snapshots"
        title="Snapshots and resilience."
        lede="Capture point-in-time state, restore in place, and keep day-2 recovery in the same console as create and power."
      >
        <VncStage />
      </MktChapter>

      <MktBand
        title="Ready when you are."
        lede="Sign in and open the console — create a VM, attach a console, expose SSH."
        cta={
          <Link to="/sign-in" className="zf-btn mkt-band-cta">
            Sign in
          </Link>
        }
      />
    </MarketingLayout>
  )
}
