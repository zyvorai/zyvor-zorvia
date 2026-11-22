// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

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

export default function SecurityPage() {
  return (
    <MarketingLayout>
      <MktHero
        compact
        kicker="Zorvia"
        title={
          <>
            Security that
            <br />
            stays out of the way.
          </>
        }
        punch="Built in."
        lede="JWT roles, audit trails, and network controls in the control plane — not bolted on later."
      />

      <MktChapter
        id="access"
        title="Access."
        lede="Sign-in with JWT. Roles for operators. API keys for automation. Optional TOTP and OIDC when you need them."
      >
        <ConsoleStage eyebrow="Auth" headline="Sign in · roles · API keys" />
      </MktChapter>

      <MktChapter
        id="network"
        title="Network security."
        lede="Expose only what you intend — NodePort SSH, VNC, and RDP are explicit Services, labeled per VM."
        inverse
      >
        <ExposeStage />
      </MktChapter>

      <MktChapter
        id="compliance"
        title="Compliance posture."
        lede="Audit-friendly APIs, sanitized errors, TLS for the console, and cluster RBAC for console and VNC subresources."
      >
        <TerminalStage />
      </MktChapter>

      <MktBand
        title="Operate with confidence."
        lede="Open the console when you are ready."
        cta={
          <Link to="/sign-in" className="zf-btn mkt-band-cta">
            Sign in
          </Link>
        }
      />
    </MarketingLayout>
  )
}
