// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { lazy, Suspense, useEffect, useState } from 'react'
import { Link, useParams, useSearchParams } from 'react-router'
import { ChevronLeft } from 'lucide-react'
import ErrorBanner from '../components/ErrorBanner'
import { listPods, type PodSummary } from '../api/pods'

const PodLogs = lazy(() => import('../components/PodLogs'))

/** Full-window log view for one pod — the "open in new tab" target from the Pods panel. */
export default function PodLogsPage() {
  const { ns = '', name = '' } = useParams()
  const [params, setParams] = useSearchParams()
  const [pod, setPod] = useState<PodSummary | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    document.title = `${name} logs · Zorvia`
    listPods(ns)
      .then((pods) => {
        const found = pods.find((p) => p.name === name)
        if (found) setPod(found)
        else setError(`Pod ${ns}/${name} was not found — it may have been deleted or replaced.`)
      })
      .catch((e) => setError(e instanceof Error ? e.message : String(e)))
  }, [ns, name])

  const names = pod
    ? [...pod.containers.filter((c) => !c.init), ...pod.containers.filter((c) => c.init)].map((c) => c.name)
    : []
  const requested = params.get('container') ?? ''
  const container = names.includes(requested) ? requested : names[0] ?? ''

  return (
    <div className="flex flex-col gap-3" style={{ height: 'calc(100vh - var(--console-topbar-height) - 48px)' }}>
      <div className="flex items-center gap-2 text-sm">
        <Link to="/app/pods" className="inline-flex items-center gap-1 text-[var(--zf-link)] hover:underline">
          <ChevronLeft className="w-4 h-4" /> Pods
        </Link>
        <span className="text-[var(--zf-tertiary)]">/</span>
        <span className="text-[var(--zf-secondary)]">{ns}</span>
        <span className="text-[var(--zf-tertiary)]">/</span>
        <span className="font-medium text-[var(--zf-ink)] truncate">{name}</span>
      </div>
      {error && <ErrorBanner title="Could not open logs" headline={error} />}
      <div className="flex-1 min-h-0" data-testid="pod-logs-page">
        {pod && (
          <Suspense fallback={<div className="zf-terminal zf-terminal-pro rounded-xl h-full" />}>
            <PodLogs
              namespace={ns}
              pod={name}
              containers={names}
              container={container}
              onContainerChange={(c) => setParams({ container: c }, { replace: true })}
            />
          </Suspense>
        )}
      </div>
    </div>
  )
}
