// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

import { ReactNode, useEffect, useMemo, useRef, useState } from 'react'
import { Link, NavLink, useLocation, useNavigate } from 'react-router'
import { LogOut, Menu, Search, X } from 'lucide-react'
import { useAuth } from '../contexts/AuthContext'
import { usePermissions } from '../hooks/usePermissions'
import { TOP_NAV, type NavItem } from '../navigation/navConfig'
import { navItemActive } from '../utils/routes'
import ConnectionStatus from './ConnectionStatus'
import CommandPalette from './CommandPalette'
import Breadcrumb from './Breadcrumb'
import HelpDialog, { type HelpTab } from './HelpDialog'
import { useSequenceShortcuts } from '../hooks/useSequenceShortcut'
import { ZyvorLockup } from './ZyvorMark'
import ThemeToggle from './ThemeToggle'
import WhatsNew from './WhatsNew'
import { useKeyboardShortcut, isInputFocused } from '../hooks/useKeyboardShortcut'
import { useRecordRecentPage } from '../hooks/useRecordRecentPage'

const OPEN_DELAY_MS = 120
const CLOSE_DELAY_MS = 450

function ConsoleShortcuts({
  helpOpen,
  helpTab,
  onOpenHelp,
  onCloseHelp,
  onHelpTabChange,
}: {
  helpOpen: boolean
  helpTab: HelpTab
  onOpenHelp: (tab?: HelpTab) => void
  onCloseHelp: () => void
  onHelpTabChange: (tab: HelpTab) => void
}) {
  const navigate = useNavigate()
  const shortcuts = useMemo(
    () => [
      { sequence: ['g', 'd'] as [string, string], handler: () => navigate('/app') },
      { sequence: ['g', 'v'] as [string, string], handler: () => navigate('/app/vms') },
      { sequence: ['g', 'c'] as [string, string], handler: () => navigate('/app/create') },
      { sequence: ['g', 'f'] as [string, string], handler: () => navigate('/app/favorites') },
      { sequence: ['g', 's'] as [string, string], handler: () => navigate('/app/snapshots') },
      { sequence: ['g', 'w'] as [string, string], handler: () => navigate('/app/windows') },
    ],
    [navigate],
  )
  useSequenceShortcuts(shortcuts)

  useKeyboardShortcut({
    key: '?',
    handler: (e) => {
      if (isInputFocused()) return
      e.preventDefault()
      if (helpOpen) onCloseHelp()
      else onOpenHelp('shortcuts')
    },
  })

  return (
    <HelpDialog open={helpOpen} tab={helpTab} onClose={onCloseHelp} onTabChange={onHelpTabChange} />
  )
}

function filterItems(items: NavItem[], canAdmin: boolean): NavItem[] {
  return items.filter((item) => !item.adminOnly || canAdmin)
}

export default function ConsoleLayout({ children }: { children: ReactNode }) {
  const { user, logout } = useAuth()
  const { canAdmin } = usePermissions()
  const navigate = useNavigate()
  const location = useLocation()
  const [mobileNav, setMobileNav] = useState(false)
  const [openGroup, setOpenGroup] = useState<string | null>(null)
  const [helpOpen, setHelpOpen] = useState(false)
  const [helpTab, setHelpTab] = useState<HelpTab>('shortcuts')
  const openTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const closeTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const navRef = useRef<HTMLElement | null>(null)
  const triggerRefs = useRef<Record<string, HTMLButtonElement | null>>({})

  useRecordRecentPage()

  const clearTimers = () => {
    if (openTimer.current) clearTimeout(openTimer.current)
    if (closeTimer.current) clearTimeout(closeTimer.current)
    openTimer.current = null
    closeTimer.current = null
  }

  const scheduleOpen = (label: string) => {
    clearTimers()
    openTimer.current = setTimeout(() => setOpenGroup(label), OPEN_DELAY_MS)
  }

  const scheduleClose = () => {
    clearTimers()
    closeTimer.current = setTimeout(() => setOpenGroup(null), CLOSE_DELAY_MS)
  }

  const toggleGroup = (label: string) => {
    clearTimers()
    setOpenGroup((cur) => (cur === label ? null : label))
  }

  useEffect(() => () => clearTimers(), [])

  useEffect(() => {
    if (!openGroup) return
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      const label = openGroup
      setOpenGroup(null)
      triggerRefs.current[label]?.focus()
    }
    const onPointerDown = (e: MouseEvent) => {
      if (navRef.current && !navRef.current.contains(e.target as Node)) setOpenGroup(null)
    }
    document.addEventListener('keydown', onKeyDown)
    document.addEventListener('mousedown', onPointerDown)
    return () => {
      document.removeEventListener('keydown', onKeyDown)
      document.removeEventListener('mousedown', onPointerDown)
    }
  }, [openGroup])

  useEffect(() => {
    setOpenGroup(null)
    setMobileNav(false)
  }, [location.pathname])

  const onLogout = () => {
    logout()
    navigate('/sign-in', { replace: true })
  }

  const closeMega = () => {
    setOpenGroup(null)
    setMobileNav(false)
  }

  return (
    <div className="console-shell">
      <a href="#main-content" className="zf-skip-link">
        Skip to main content
      </a>
      <ConsoleShortcuts
        helpOpen={helpOpen}
        helpTab={helpTab}
        onOpenHelp={(tab = 'shortcuts') => {
          setHelpTab(tab)
          setHelpOpen(true)
        }}
        onCloseHelp={() => setHelpOpen(false)}
        onHelpTabChange={setHelpTab}
      />
      <CommandPalette
        onOpenHelp={(tab) => {
          setHelpTab(tab ?? 'shortcuts')
          setHelpOpen(true)
        }}
      />

      <header className="console-topbar" ref={navRef}>
        <button
          type="button"
          className="console-icon-btn lg:hidden"
          onClick={() => {
            setMobileNav((v) => !v)
            setOpenGroup(null)
          }}
          aria-label="Toggle navigation"
          aria-expanded={mobileNav}
        >
          {mobileNav ? <X className="w-4 h-4" /> : <Menu className="w-4 h-4" />}
        </button>
        <Link to="/app" className="console-brand" aria-label="Zorvia" onClick={closeMega}>
          <ZyvorLockup markClassName="w-6 h-6" showWordmark={false} />
          <span className="console-brand-word">Zorvia</span>
        </Link>

        <nav className={`console-navlinks ${mobileNav ? 'mobile-open' : ''}`} aria-label="Console">
          {TOP_NAV.map((entry) => {
            if (entry.kind === 'link') {
              const item = entry.item
              if (item.adminOnly && !canAdmin) return null
              return (
                <NavLink
                  key={item.path}
                  to={item.path}
                  end={item.path === '/app'}
                  className={({ isActive }) => `console-nav-trigger${isActive ? ' active' : ''}`}
                  onClick={closeMega}
                >
                  {item.label}
                </NavLink>
              )
            }
            const items = filterItems(entry.items, canAdmin)
            if (items.length === 0) return null
            const groupActive = items.some((item) =>
              navItemActive(item, location.pathname, location.search),
            )
            const isOpen = openGroup === entry.label
            return (
              <div
                key={entry.label}
                className="console-navgroup"
                onMouseEnter={() => {
                  if (window.matchMedia('(min-width: 1024px)').matches) scheduleOpen(entry.label)
                }}
                onMouseLeave={() => {
                  if (window.matchMedia('(min-width: 1024px)').matches) scheduleClose()
                }}
              >
                <button
                  type="button"
                  ref={(el) => {
                    triggerRefs.current[entry.label] = el
                  }}
                  className={`console-nav-trigger${groupActive ? ' active' : ''}`}
                  aria-haspopup="true"
                  aria-expanded={isOpen}
                  onClick={() => toggleGroup(entry.label)}
                >
                  {entry.label}
                </button>
                <div
                  className={`console-mega-panel${isOpen ? ' open' : ''}`}
                  role="region"
                  aria-label={entry.label}
                  onMouseEnter={() => scheduleOpen(entry.label)}
                  onMouseLeave={scheduleClose}
                >
                  <div className="console-mega-grid">
                    {items.map((item) => {
                      const active = navItemActive(item, location.pathname, location.search)
                      return (
                        <NavLink
                          key={item.path}
                          to={item.path}
                          className={`console-mega-link${active ? ' active' : ''}`}
                          aria-current={active ? 'page' : undefined}
                          onClick={closeMega}
                        >
                          <span className="console-mega-link-label">{item.label}</span>
                          {item.blurb && (
                            <span className="console-mega-link-blurb">{item.blurb}</span>
                          )}
                        </NavLink>
                      )
                    })}
                  </div>
                </div>
              </div>
            )
          })}
        </nav>

        <div className="console-nav-actions">
          <button
            type="button"
            className="zf-btn zf-btn-ghost zf-btn-sm hidden sm:inline-flex"
            onClick={() =>
              document.dispatchEvent(new KeyboardEvent('keydown', { key: 'k', metaKey: true }))
            }
          >
            <Search className="w-3.5 h-3.5" />
            Search
            <kbd className="text-[10px] text-[var(--zf-muted)] ml-1">⌘K</kbd>
          </button>
          <ConnectionStatus />
          <WhatsNew />
          <ThemeToggle />
          <Link
            to="/"
            className="text-xs text-[var(--zf-muted)] hidden md:inline hover:text-[var(--zf-ink)]"
          >
            Site
          </Link>
          <span className="text-xs text-[var(--zf-muted)] hidden sm:inline">{user?.username}</span>
          <button
            type="button"
            className="console-icon-btn"
            onClick={onLogout}
            title="Sign out"
            aria-label="Log out"
          >
            <LogOut className="w-3.5 h-3.5" />
          </button>
        </div>
      </header>

      <div className="console-body">
        <main id="main-content" className="console-main" tabIndex={-1}>
          <Breadcrumb />
          {children}
        </main>
      </div>
    </div>
  )
}
