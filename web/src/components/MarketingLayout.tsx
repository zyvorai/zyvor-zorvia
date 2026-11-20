// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { ReactNode } from 'react'
import { Link } from 'react-router'
import { useAuth } from '../contexts/AuthContext'
import { ZyvorLockup } from './ZyvorMark'
import ThemeToggle from './ThemeToggle'

const LINKS = [
  { label: 'Product', to: '/product' },
  { label: 'Platform', to: '/platform' },
  { label: 'Security', to: '/security' },
]

export default function MarketingLayout({
  children,
  subnav,
}: {
  children: ReactNode
  /** Optional local chapter links (e.g. Product overview anchors). */
  subnav?: { label: string; href: string }[]
}) {
  const { isAuthenticated } = useAuth()

  return (
    <div className="min-h-screen bg-[var(--zf-canvas)] text-[var(--zf-ink)]">
      <header className="mkt-nav">
        <div className="mkt-nav-inner">
          <nav className="flex items-center gap-8 min-w-0">
            <Link to="/" className="mkt-brand" aria-label="Zorvia home">
              <ZyvorLockup markClassName="w-6 h-6" />
              <span>Zorvia</span>
            </Link>
            <div className="mkt-nav-links">
              {LINKS.map((l) => (
                <Link key={l.to} to={l.to}>
                  {l.label}
                </Link>
              ))}
            </div>
          </nav>
          <div className="flex items-center gap-5 shrink-0">
            <ThemeToggle />
            {isAuthenticated ? (
              <Link to="/app" className="font-medium">
                Open console
              </Link>
            ) : (
              <Link to="/sign-in" className="font-medium">
                Sign in
              </Link>
            )}
          </div>
        </div>
      </header>
      {subnav && subnav.length > 0 ? (
        <nav className="mkt-subnav" aria-label="On this page">
          <div className="mkt-subnav-inner">
            {subnav.map((item) => (
              <a key={item.href} href={item.href}>
                {item.label}
              </a>
            ))}
          </div>
        </nav>
      ) : null}
      {children}
      <footer className="mkt-footer">
        <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-3">
          <span>Copyright © {new Date().getFullYear()} ZyvorAI Labs. All rights reserved.</span>
          <div className="flex gap-5">
            <Link to="/product">Product</Link>
            <Link to="/platform">Platform</Link>
            <Link to="/security">Security</Link>
            <a href="https://zyvor.dev" target="_blank" rel="noreferrer">
              zyvor.dev
            </a>
          </div>
        </div>
      </footer>
    </div>
  )
}
