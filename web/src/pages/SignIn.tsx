// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

import { FormEvent, useEffect, useMemo, useState } from 'react'
import { Link, Navigate, useNavigate } from 'react-router'
import {
  ArrowRight,
  ChevronLeft,
  Eye,
  EyeOff,
  Loader2,
  Lock,
  User,
} from 'lucide-react'
import { useAuth } from '../contexts/AuthContext'
import { formatUserError } from '../utils/apiError'
import { ZyvorLockup } from '../components/ZyvorMark'
import {
  PremiumLoginShell,
  LoginError,
  LoginField,
  LoginSubmit,
  LoginRemember,
} from '../components/PremiumLoginShell'

type LoginStep = 'identify' | 'password'

export type FabricInstanceInfo = {
  product: string
  product_id: string
  version: string
  hostname: string
  deploy_mode: string
  deploy_label: string
  kubernetes: boolean
  kubernetes_namespace?: string | null
  listen?: string | null
}

const SAVE_KEY = 'zorvia-saved-login'

function inferDeployFromBrowser(): Pick<
  FabricInstanceInfo,
  'deploy_mode' | 'deploy_label' | 'kubernetes'
> {
  const port = window.location.port
  if (port === '30152' || port === '30095') {
    return {
      deploy_mode: 'kubernetes',
      deploy_label: `Kubernetes · NodePort ${port || '30152'}`,
      kubernetes: true,
    }
  }
  if (port === '5151' || port === '9095' || port === '') {
    return {
      deploy_mode: 'host',
      deploy_label: `Host service · :${port || '5151'}`,
      kubernetes: false,
    }
  }
  return {
    deploy_mode: 'unknown',
    deploy_label: `Web · :${port || 'default'}`,
    kubernetes: false,
  }
}

async function fetchInstance(): Promise<FabricInstanceInfo> {
  const fallbackHost = window.location.hostname || 'localhost'
  const inferred = inferDeployFromBrowser()
  try {
    const res = await fetch('/api/v1/instance', { credentials: 'same-origin' })
    if (!res.ok) throw new Error(`HTTP ${res.status}`)
    const data = (await res.json()) as Partial<FabricInstanceInfo>
    return {
      product: data.product || 'Zorvia',
      product_id: data.product_id || 'zorvia',
      version: data.version || '0.2.0',
      hostname: data.hostname || fallbackHost,
      deploy_mode: data.deploy_mode || inferred.deploy_mode,
      deploy_label: data.deploy_label || inferred.deploy_label,
      kubernetes: Boolean(data.kubernetes ?? inferred.kubernetes),
      kubernetes_namespace: data.kubernetes_namespace ?? null,
      listen: data.listen ?? null,
    }
  } catch {
    return {
      product: 'Zorvia',
      product_id: 'zorvia',
      version: '0.2.0',
      hostname: fallbackHost,
      ...inferred,
      kubernetes_namespace: null,
      listen: null,
    }
  }
}

function PasswordHelp({ instance, open }: { instance: FabricInstanceInfo | null; open: boolean }) {
  const [copied, setCopied] = useState(false)
  const ns = instance?.kubernetes_namespace || 'zorvia-system'
  const command = `kubectl -n ${ns} get secret zorvia-auth -o jsonpath='{.data.admin-password}' | base64 -d; echo`

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(command)
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1800)
    } catch {
      // Clipboard can be blocked on plain HTTP; the command is still selectable.
    }
  }

  return (
    <details className="login-help" open={open} style={{ marginTop: 14, fontSize: 13, color: '#6e6e73' }}>
      <summary style={{ cursor: 'pointer', fontWeight: 500 }}>Where do I find the password?</summary>
      {instance?.kubernetes === false ? (
        <p style={{ margin: '8px 0 0' }}>
          The username is <span className="font-mono">admin</span>. The password is the value of{' '}
          <span className="font-mono">ZORVIA_ADMIN_PASSWORD</span>. If it was not set on a fresh database, a generated one was
          written to <span className="font-mono">bootstrap-admin-password</span> next to the auth database. Read it once, then
          delete the file.
        </p>
      ) : (
        <>
          <p style={{ margin: '8px 0 6px' }}>
            The username is <span className="font-mono">admin</span>. The password is generated at install time and stored in the{' '}
            <span className="font-mono">zorvia-auth</span> secret. Run this on a host with cluster access:
          </p>
          <div style={{ display: 'flex', gap: 8, alignItems: 'flex-start' }}>
            <code
              className="font-mono"
              style={{ flex: 1, padding: '8px 10px', borderRadius: 8, background: '#f2f2f4', userSelect: 'all', overflowWrap: 'anywhere', fontSize: 12 }}
            >
              {command}
            </code>
            <button
              type="button"
              onClick={copy}
              style={{ padding: '6px 10px', borderRadius: 8, border: '1px solid #d2d2d7', background: '#fff', cursor: 'pointer', fontSize: 12 }}
            >
              {copied ? 'Copied' : 'Copy'}
            </button>
          </div>
        </>
      )}
    </details>
  )
}

export default function SignIn() {
  const { login, isAuthenticated, loading } = useAuth()
  const navigate = useNavigate()

  const saved = (() => {
    try {
      const raw = localStorage.getItem(SAVE_KEY)
      if (!raw) return null
      const parsed = JSON.parse(raw) as { username?: string }
      return { username: parsed?.username || '' }
    } catch {
      return null
    }
  })()

  const [step, setStep] = useState<LoginStep>(saved?.username ? 'password' : 'identify')
  const [username, setUsername] = useState(saved?.username || '')
  const [password, setPassword] = useState('')
  const [showPassword, setShowPassword] = useState(false)
  const [rememberMe, setRememberMe] = useState(!!saved?.username)
  const [error, setError] = useState<string | null>(null)
  const [submitting, setSubmitting] = useState(false)
  const [instance, setInstance] = useState<FabricInstanceInfo | null>(null)

  useEffect(() => {
    let alive = true
    fetchInstance().then((info) => {
      if (alive) setInstance(info)
    })
    return () => {
      alive = false
    }
  }, [])

  const pills = useMemo(() => {
    const base = [
      { label: 'Zyvor', tone: 'orange' as const },
      { label: 'KubeVirt', tone: 'emerald' as const },
      { label: 'Kubernetes', tone: 'violet' as const },
      { label: 'CLI + Web', tone: 'sky' as const },
    ]
    if (instance?.kubernetes) {
      return [{ label: 'NodePort', tone: 'sky' as const }, ...base]
    }
    return [{ label: 'Bare metal', tone: 'amber' as const }, ...base]
  }, [instance?.kubernetes])

  if (!loading && isAuthenticated) {
    return <Navigate to="/app" replace />
  }

  const scrollToForm = () => {
    document.getElementById('login-sign-in')?.scrollIntoView({ behavior: 'smooth', block: 'start' })
  }

  const handleContinue = (e: FormEvent) => {
    e.preventDefault()
    if (!username.trim()) return
    setError(null)
    setStep('password')
  }

  const handleBack = () => {
    setStep('identify')
    setPassword('')
    setShowPassword(false)
    setError(null)
  }

  const onSubmit = async (e: FormEvent) => {
    e.preventDefault()
    if (!username.trim() || !password) return
    setError(null)
    setSubmitting(true)
    try {
      await login(username.trim(), password)
      if (rememberMe) {
        localStorage.setItem(SAVE_KEY, JSON.stringify({ username: username.trim() }))
      } else {
        localStorage.removeItem(SAVE_KEY)
      }
      navigate('/app', { replace: true })
    } catch (err) {
      setError(formatUserError(err))
    } finally {
      setSubmitting(false)
    }
  }

  const instanceMeta = instance ? (
    <>
      <span className="login-instance-chip" data-kind="product">
        <strong>{instance.product}</strong>
      </span>
      <span className="login-instance-chip" title="Host">
        {instance.hostname}
      </span>
      <span
        className="login-instance-chip"
        data-kind={instance.kubernetes ? 'k8s' : undefined}
        title="Deploy mode"
      >
        {instance.deploy_label}
      </span>
      {instance.kubernetes_namespace ? (
        <span className="login-instance-chip" data-kind="k8s" title="Namespace">
          ns/{instance.kubernetes_namespace}
        </span>
      ) : null}
      <span className="login-instance-chip" title="Version">
        v{instance.version}
      </span>
    </>
  ) : null

  const formInstance = instance ? (
    <dl className="login-form-instance">
      <dt>Project</dt>
      <dd>{instance.product}</dd>
      <dt>System</dt>
      <dd>
        {instance.hostname}
        {instance.listen ? ` · ${instance.listen}` : ''}
      </dd>
      <dt>Deploy</dt>
      <dd>
        {instance.deploy_label}
        {instance.kubernetes_namespace ? ` · ${instance.kubernetes_namespace}` : ''}
      </dd>
    </dl>
  ) : null

  const storeCta = (
    <>
      <a
        href="#login-sign-in"
        className="login-cta-primary"
        onClick={(e) => {
          e.preventDefault()
          scrollToForm()
        }}
      >
        Sign in
      </a>
      <Link to="/" className="login-cta-secondary">
        Learn more
      </Link>
    </>
  )

  const panelSubtitle =
    step === 'password' ? (
      <>
        Enter the password for <span className="login-apple-host">{username.trim()}</span>
      </>
    ) : (
      'Sign in to Zorvia'
    )

  const chapterNote = instance
    ? `${instance.product} · ${instance.deploy_label} · ${instance.hostname}`
    : 'Zorvia · sign in to continue'

  return (
    <PremiumLoginShell
      logo={
        <a
          href="https://zyvor.dev"
          target="_blank"
          rel="noopener noreferrer"
          className="login-zyvor-mark"
          aria-label="zyvor.dev"
          title="zyvor.dev"
        >
          <span className="inline-flex items-center gap-2">
            <ZyvorLockup
              markClassName="w-9 h-9"
              wordmarkClassName="text-[1.35rem] text-[#1d1d1f]"
            />
            <span className="text-[1.15rem] font-medium text-[#6e6e73] tracking-[-0.02em] leading-none">
              Zorvia
            </span>
          </span>
        </a>
      }
      productName="Zorvia"
      productWordmark=""
      heroTitle="Craft VMs. One console."
      heroSubheadline="KubeVirt VMs for private cloud — manage the fleet from one control plane."
      accent="orange"
      pills={pills}
      instanceMeta={instanceMeta}
      heroCta={storeCta}
      chapterNote={chapterNote}
      panelSubtitle={panelSubtitle}
      footer={
        <p className="login-hint" style={{ marginTop: 0, marginBottom: '1.25rem' }}>
          © 2026 Zyvor ·{' '}
          <a
            href="https://zyvor.dev"
            target="_blank"
            rel="noopener noreferrer"
            className="login-zyvor-link"
          >
            zyvor.dev
          </a>
        </p>
      }
      showSignInChapter
    >
      {step === 'identify' ? (
        <form
          key="identify"
          onSubmit={handleContinue}
          autoComplete="on"
          aria-label="Account"
          className="login-apple-step text-left"
          noValidate
        >
          {formInstance}
          {error ? <LoginError message={error} /> : null}
          <div className="login-apple-fields">
            <LoginField label="Username" id="username">
              <User className="login-field-icon" />
              <input
                id="username"
                name="username"
                type="text"
                value={username}
                onChange={(e) => setUsername(e.target.value)}
                className="login-input"
                placeholder="admin"
                autoComplete="username"
                autoFocus
                required
              />
            </LoginField>
          </div>
          <LoginSubmit loading={false} disabled={!username.trim()}>
            <span>Continue</span>
            <ArrowRight className="h-4 w-4" />
          </LoginSubmit>
          <PasswordHelp instance={instance} open />
        </form>
      ) : (
        <form
          key="password"
          onSubmit={onSubmit}
          autoComplete="on"
          aria-label="Password"
          className="login-apple-step text-left"
          noValidate
        >
          <button type="button" onClick={handleBack} className="login-apple-identity" aria-label="Change account">
            <ChevronLeft aria-hidden className="h-4 w-4 shrink-0" />
            <span className="truncate">{username.trim()}</span>
          </button>
          <input type="text" name="username" value={username} autoComplete="username" readOnly hidden />
          {formInstance}
          {error ? <LoginError message={error} /> : null}
          <div className="login-apple-fields">
            <LoginField label="Password" id="password">
              <Lock className="login-field-icon" />
              <input
                id="password"
                name="password"
                type={showPassword ? 'text' : 'password'}
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                className="login-input pr-11"
                placeholder="Password"
                autoComplete="current-password"
                autoFocus
                required
                disabled={submitting}
              />
              <button
                type="button"
                onClick={() => setShowPassword(!showPassword)}
                className="absolute right-3.5 top-1/2 -translate-y-1/2 text-zinc-400 hover:text-zinc-700 transition-colors"
                aria-label={showPassword ? 'Hide password' : 'Show password'}
              >
                {showPassword ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
              </button>
            </LoginField>
          </div>
          <LoginRemember
            checked={rememberMe}
            onChange={(checked) => {
              setRememberMe(checked)
              if (!checked) localStorage.removeItem(SAVE_KEY)
            }}
            label="Remember me on this device"
          />
          <LoginSubmit loading={submitting} disabled={!password}>
            {submitting ? (
              <>
                <Loader2 className="h-4 w-4 animate-spin" />
                <span>Signing in…</span>
              </>
            ) : (
              <>
                <span>Sign In</span>
                <ArrowRight className="h-4 w-4" />
              </>
            )}
          </LoginSubmit>
          <PasswordHelp instance={instance} open={false} />
        </form>
      )}
    </PremiumLoginShell>
  )
}
