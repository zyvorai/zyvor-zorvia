// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { useCallback, useEffect, useState } from 'react'
import { RotateCw } from 'lucide-react'
import { AppleTerminalFrame } from './AppleTerminalFrame'
import { podEvents, type PodEvent } from '../api/pods'
import { TERM_HEX } from './terminalTheme'

const REFRESH_MS = 5_000

function ago(ts: string | null): string {
  if (!ts) return '—'
  const secs = Math.max(0, Math.round((Date.now() - new Date(ts).getTime()) / 1000))
  if (secs < 60) return `${secs}s`
  if (secs < 3600) return `${Math.floor(secs / 60)}m`
  if (secs < 86_400) return `${Math.floor(secs / 3600)}h`
  return `${Math.floor(secs / 86_400)}d`
}

export default function PodEvents({ namespace, pod }: { namespace: string; pod: string }) {
  const [events, setEvents] = useState<PodEvent[]>([])
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)

  const load = useCallback(async () => {
    try {
      setEvents(await podEvents(namespace, pod))
      setError(null)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setLoading(false)
    }
  }, [namespace, pod])

  useEffect(() => {
    setLoading(true)
    void load()
    const t = window.setInterval(() => void load(), REFRESH_MS)
    return () => window.clearInterval(t)
  }, [load])

  const warnings = events.filter((e) => e.type === 'Warning').length

  return (
    <div data-testid="pod-events" className="h-full">
      <AppleTerminalFrame
        title={`events · ${namespace}/${pod}`}
        live
        className="zf-terminal-pro flex flex-col h-full"
        bodyClassName="flex-1 min-h-0 overflow-auto px-4 py-3"
        trailing={
          <button type="button" className="zf-term-btn" onClick={() => void load()} aria-label="Refresh events">
            <RotateCw className="w-3.5 h-3.5" />
          </button>
        }
      >
        <div className="whitespace-pre" style={{ color: TERM_HEX.fg }}>
          <div>
            <span style={{ color: TERM_HEX.green }}>zorvia@pods</span>
            <span style={{ color: TERM_HEX.dim }}>:</span>
            <span style={{ color: TERM_HEX.blue }}>~</span>
            <span style={{ color: TERM_HEX.dim }}>$ </span>
            kubectl events -n {namespace} --for pod/{pod}
          </div>
          {error ? (
            <div style={{ color: TERM_HEX.red }}>error: {error}</div>
          ) : loading && events.length === 0 ? (
            <div style={{ color: TERM_HEX.dim }}>Loading events…</div>
          ) : events.length === 0 ? (
            <div style={{ color: TERM_HEX.dim }}>No events (Kubernetes keeps them for about an hour).</div>
          ) : (
            <>
              <div style={{ color: TERM_HEX.dim }}>
                {events.length} events{warnings > 0 && ` · ${warnings} warning${warnings === 1 ? '' : 's'}`}
              </div>
              <table className="zf-ls mt-1 border-collapse">
                <thead>
                  <tr style={{ color: TERM_HEX.dim }}>
                    <th>LAST SEEN</th>
                    <th>TYPE</th>
                    <th>REASON</th>
                    <th>SOURCE</th>
                    <th>MESSAGE</th>
                  </tr>
                </thead>
                <tbody>
                  {events.map((e, i) => {
                    const warn = e.type === 'Warning'
                    return (
                      <tr key={`${e.reason}-${e.last_seen}-${i}`} className="zf-ls-static">
                        <td style={{ color: TERM_HEX.dim }}>{ago(e.last_seen)}</td>
                        <td style={{ color: warn ? TERM_HEX.yellow : TERM_HEX.green }}>{e.type}</td>
                        <td className="font-semibold" style={{ color: warn ? TERM_HEX.orange : TERM_HEX.fg }}>
                          {e.reason}
                          {e.count > 1 && <span style={{ color: TERM_HEX.magenta }}> ×{e.count}</span>}
                        </td>
                        <td style={{ color: TERM_HEX.blue }}>{e.source || '—'}</td>
                        <td className="whitespace-pre-wrap" style={{ color: warn ? TERM_HEX.yellow : TERM_HEX.fg }}>
                          {e.message}
                        </td>
                      </tr>
                    )
                  })}
                </tbody>
              </table>
            </>
          )}
        </div>
      </AppleTerminalFrame>
    </div>
  )
}
