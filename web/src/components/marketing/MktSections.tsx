// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import {
  ReactNode,
  useEffect,
  useRef,
  useState,
  type KeyboardEvent,
} from 'react'
import { Link } from 'react-router'

function useInViewOnce<T extends HTMLElement>() {
  const ref = useRef<T | null>(null)
  const [inView, setInView] = useState(false)

  useEffect(() => {
    const el = ref.current
    if (!el || inView) return
    if (typeof IntersectionObserver === 'undefined') {
      setInView(true)
      return
    }
    const io = new IntersectionObserver(
      ([entry]) => {
        if (entry?.isIntersecting) {
          setInView(true)
          io.disconnect()
        }
      },
      { threshold: 0.2 },
    )
    io.observe(el)
    return () => io.disconnect()
  }, [inView])

  return { ref, inView }
}

export function MktHero({
  kicker = 'Zorvia',
  title,
  punch,
  lede,
  cta,
  visual,
  compact = false,
}: {
  kicker?: string
  title: ReactNode
  punch?: ReactNode
  lede?: ReactNode
  cta?: ReactNode
  visual?: ReactNode
  compact?: boolean
}) {
  const { ref, inView } = useInViewOnce<HTMLDivElement>()

  return (
    <section className={`mkt-hero${compact ? ' mkt-hero--compact' : ''}`}>
      <div className="mkt-hero-copy">
        <p className="mkt-kicker mkt-reveal">{kicker}</p>
        <h1 className="mkt-reveal">{title}</h1>
        {punch ? <p className="mkt-punch mkt-reveal-delay">{punch}</p> : null}
        {lede ? <p className="lede mkt-reveal-delay">{lede}</p> : null}
        {cta ? <div className="mkt-cta-row mkt-reveal-delay-2">{cta}</div> : null}
      </div>
      {visual ? (
        <div
          ref={ref}
          className={`mkt-hero-visual${inView ? ' is-inview' : ''}`}
        >
          <div className="mkt-product-stage">{visual}</div>
        </div>
      ) : null}
    </section>
  )
}

export function MktChapter({
  id,
  title,
  lede,
  children,
  inverse = false,
}: {
  id?: string
  title: ReactNode
  lede?: ReactNode
  children?: ReactNode
  inverse?: boolean
}) {
  return (
    <section
      id={id}
      className={`mkt-chapter${inverse ? ' mkt-chapter--inverse' : ''}`}
    >
      <div className="mkt-chapter-inner">
        <h2>{title}</h2>
        {lede ? <p className="lede">{lede}</p> : null}
        {children ? <div className="mkt-chapter-visual">{children}</div> : null}
      </div>
    </section>
  )
}

export function MktBand({
  title,
  lede,
  cta,
}: {
  title: ReactNode
  lede?: ReactNode
  cta?: ReactNode
}) {
  return (
    <section className="mkt-band">
      <h2>{title}</h2>
      {lede ? <p>{lede}</p> : null}
      {cta}
    </section>
  )
}

export type MktHighlightItem = {
  id: string
  label: string
  title: string
  body: string
  visual: ReactNode
}

export function MktHighlights({
  heading = 'Get the highlights.',
  items,
}: {
  heading?: string
  items: MktHighlightItem[]
}) {
  const [active, setActive] = useState(items[0]?.id ?? '')
  const current = items.find((i) => i.id === active) ?? items[0]

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const idx = items.findIndex((i) => i.id === active)
    if (idx < 0) return
    if (e.key === 'ArrowRight') {
      e.preventDefault()
      setActive(items[(idx + 1) % items.length].id)
    } else if (e.key === 'ArrowLeft') {
      e.preventDefault()
      setActive(items[(idx - 1 + items.length) % items.length].id)
    }
  }

  if (!current) return null

  return (
    <section className="mkt-highlights">
      <div className="mkt-highlights-inner">
        <h2>{heading}</h2>
        <div
          className="mkt-highlight-tabs"
          role="tablist"
          aria-label="Highlights"
          onKeyDown={onKeyDown}
        >
          {items.map((item) => (
            <button
              key={item.id}
              type="button"
              role="tab"
              id={`tab-${item.id}`}
              aria-selected={item.id === current.id}
              aria-controls={`panel-${item.id}`}
              className="mkt-highlight-tab"
              onClick={() => setActive(item.id)}
            >
              {item.label}
            </button>
          ))}
        </div>
        <div
          className="mkt-highlight-panel"
          role="tabpanel"
          id={`panel-${current.id}`}
          aria-labelledby={`tab-${current.id}`}
        >
          <h3>{current.title}</h3>
          <p>{current.body}</p>
          {current.visual}
        </div>
      </div>
    </section>
  )
}

export function ConsoleStage({
  eyebrow = 'Console',
  headline = 'Fleet. Network. Storage.',
}: {
  eyebrow?: string
  headline?: string
}) {
  return (
    <div className="mkt-stage-console" aria-hidden>
      <div className="mkt-stage-chrome">
        <span className="mkt-stage-dot" />
        <span className="mkt-stage-dot" />
        <span className="mkt-stage-dot" />
        <span className="mkt-stage-title">zorvia · /app</span>
      </div>
      <div className="mkt-stage-body">
        <div className="mkt-stage-side">
          <div className="mkt-stage-side-line" />
          <div className="mkt-stage-side-line" />
          <div className="mkt-stage-side-line" />
          <div className="mkt-stage-side-line" />
        </div>
        <div className="mkt-stage-main">
          <div className="eyebrow">{eyebrow}</div>
          <div className="headline">{headline}</div>
          <div className="mkt-stage-rows">
            {[
              ['demo-linux', 'Running', '1 vCPU', '10.42.0.45'],
              ['web-01', 'Running', '4 vCPU', '10.42.0.18'],
              ['db-prod', 'Stopped', '6 vCPU', '—'],
            ].map(([name, state, cpu, ip]) => (
              <div key={name} className="mkt-stage-row">
                <span>{name}</span>
                <span className="mkt-stage-pill">{state}</span>
                <span>{cpu}</span>
                <span>{ip}</span>
              </div>
            ))}
          </div>
        </div>
      </div>
    </div>
  )
}

export function TerminalStage() {
  return (
    <div className="mkt-stage-terminal" aria-hidden>
      <div className="mkt-stage-chrome">
        <span className="mkt-stage-dot" />
        <span className="mkt-stage-dot" />
        <span className="mkt-stage-dot" />
        <span className="mkt-stage-title">demo-linux — tty</span>
      </div>
      <pre>
        <span className="ok">Connected to VM console</span>
        {'\n'}
        <span className="dim">ubuntu@demo-linux:~$</span> uname -a{'\n'}
        Linux demo-linux 6.8.0-kvm #1 SMP{'\n'}
        <span className="dim">ubuntu@demo-linux:~$</span> _
      </pre>
    </div>
  )
}

export function VncStage() {
  return (
    <div className="mkt-stage-vnc" aria-hidden>
      <div className="mkt-stage-vnc-frame">Graphical console · VNC</div>
    </div>
  )
}

export function ExposeStage() {
  return (
    <div className="mkt-stage-terminal" aria-hidden>
      <div className="mkt-stage-chrome">
        <span className="mkt-stage-dot" />
        <span className="mkt-stage-dot" />
        <span className="mkt-stage-dot" />
        <span className="mkt-stage-title">expose · NodePort</span>
      </div>
      <pre>
        <span className="dim"># SSH to guest via node</span>
        {'\n'}
        ssh zorvia@host.example -p 30247{'\n'}
        {'\n'}
        <span className="ok">Service</span> demo-linux-p22-tcp{'\n'}
        <span className="dim">guest 22 → NodePort 30247</span>
      </pre>
    </div>
  )
}

export function AuthCtas({ isAuthenticated }: { isAuthenticated: boolean }) {
  return (
    <>
      {isAuthenticated ? (
        <Link to="/app" className="zf-btn zf-btn-primary">
          Open console
        </Link>
      ) : (
        <Link to="/sign-in" className="zf-btn zf-btn-primary">
          Sign in
        </Link>
      )}
      <Link to="/product" className="zf-btn zf-btn-secondary">
        Learn more
      </Link>
    </>
  )
}
