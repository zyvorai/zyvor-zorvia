// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { useState, useEffect, useRef, useCallback } from 'react'
import { Pause, Play, Trash2 } from 'lucide-react'
import { useEventStream, type VMEventPayload } from '../hooks/useEventStream'
import ErrorBanner from '../components/ErrorBanner'
import { PageHeader } from '../components/ui'
import { hintsForError } from '../utils/daemonHints'
import { AppleTerminalFrame, TERM_LEVEL_COLOR } from '../components/AppleTerminalFrame'
import { AnsiText } from '../components/AnsiText'
import { TERM_HEX } from '../components/terminalTheme'

interface StreamEvent {
  id: number
  timestamp: Date
  type: string
  source: string
  message: string
  level: 'info' | 'warning' | 'error' | 'debug'
}

function levelColor(level: string): string {
  return TERM_LEVEL_COLOR[level] || '#64d2ff'
}

function mapPayload(payload: VMEventPayload): StreamEvent {
  const levelRaw = payload.event_type.toLowerCase()
  const level: StreamEvent['level'] =
    levelRaw.includes('error') || levelRaw.includes('fail')
      ? 'error'
      : levelRaw.includes('warn')
        ? 'warning'
        : levelRaw.includes('debug')
          ? 'debug'
          : 'info'
  return {
    id: ++eventIdCounter,
    timestamp: new Date(payload.timestamp),
    type: payload.event_type,
    source: payload.vm_name,
    message: payload.detail || payload.event_type,
    level,
  }
}

let eventIdCounter = 0

export default function EventStream() {
  const [events, setEvents] = useState<StreamEvent[]>([])
  const [paused, setPaused] = useState(false)
  const [levelFilter, setLevelFilter] = useState<string>('all')
  const [connectionError, setConnectionError] = useState<string | null>(null)
  const containerRef = useRef<HTMLDivElement>(null)
  const pausedRef = useRef(false)

  useEffect(() => { pausedRef.current = paused }, [paused])

  const onEvent = useCallback((payload: VMEventPayload) => {
    if (pausedRef.current) return
    setEvents((prev) => [mapPayload(payload), ...prev].slice(0, 500))
    setConnectionError(null)
  }, [])

  const { connected } = useEventStream({ onEvent })

  useEffect(() => {
    if (!connected && events.length === 0) {
      setConnectionError('Connecting to event stream…')
    } else if (connected) {
      setConnectionError(null)
    }
  }, [connected, events.length])

  useEffect(() => {
    if (!paused && containerRef.current) {
      containerRef.current.scrollTop = 0
    }
  }, [events, paused])

  const filtered = events.filter((e) => levelFilter === 'all' || e.level === levelFilter)

  return (
    <div className="space-y-6">
      <PageHeader
        title="Event Stream"
        description="Live VM lifecycle events via authenticated SSE"
      />

      {connectionError && !connected && (
        <ErrorBanner title="Connection error" headline={connectionError} hints={hintsForError(connectionError)} />
      )}

      <AppleTerminalFrame
        title="events — zorvia — SSE"
        live={connected && !paused}
        className="zf-terminal-pro"
        bodyRef={containerRef}
        bodyClassName="max-h-[70vh] min-h-[22rem] overflow-y-auto px-4 py-3"
        trailing={
          <div className="flex items-center gap-1.5 shrink-0" data-testid="event-stream-controls">
            {!connected && <span className="text-[11px] mr-1 text-[#ffd60a]">Reconnecting…</span>}
            <select
              value={levelFilter}
              onChange={(e) => setLevelFilter(e.target.value)}
              className="zf-term-btn zf-term-select"
              aria-label="Filter by level"
            >
              <option value="all">all levels</option>
              <option value="info">info</option>
              <option value="warning">warning</option>
              <option value="error">error</option>
              <option value="debug">debug</option>
            </select>
            <button
              type="button"
              onClick={() => setPaused(!paused)}
              className={`zf-term-btn${paused ? ' is-on' : ''}`}
              aria-label={paused ? 'Resume' : 'Pause'}
              title={paused ? 'Resume' : 'Pause'}
            >
              {paused ? <Play className="w-3.5 h-3.5" /> : <Pause className="w-3.5 h-3.5" />}
            </button>
            <button type="button" onClick={() => setEvents([])} className="zf-term-btn" aria-label="Clear" title="Clear">
              <Trash2 className="w-3.5 h-3.5" />
            </button>
          </div>
        }
      >
        <div className="text-[#f2f2f2]" data-testid="event-stream-terminal">
          <div className="whitespace-pre">
            <span style={{ color: TERM_HEX.green }}>zorvia@events</span>
            <span style={{ color: TERM_HEX.dim }}>:</span>
            <span style={{ color: TERM_HEX.blue }}>~</span>
            <span style={{ color: TERM_HEX.dim }}>$ </span>
            curl -sN /api/events/stream
            {levelFilter !== 'all' && (
              <>
                <span style={{ color: TERM_HEX.dim }}> | </span>grep -i{' '}
                <span style={{ color: TERM_HEX.yellow }}>{levelFilter}</span>
              </>
            )}
          </div>
          <div style={{ color: TERM_HEX.dim }}>
            {filtered.length} event{filtered.length === 1 ? '' : 's'}
            {paused && ' · paused'} · newest first
          </div>
          {filtered.length === 0 ? (
            <div className="mt-1" style={{ color: TERM_HEX.dim }}>
              Waiting for events<span className="zf-term-caret ml-1" />
            </div>
          ) : (
            filtered.map((ev) => (
              <div key={ev.id} className="flex gap-3 py-0.5 hover:bg-white/[0.06] rounded px-1 -mx-1">
                <span className="shrink-0 tabular-nums" style={{ color: TERM_HEX.dim }}>
                  {ev.timestamp.toLocaleTimeString(undefined, { hour12: false })}
                </span>
                <span className="shrink-0 uppercase font-semibold w-16" style={{ color: levelColor(ev.level) }}>
                  {ev.level}
                </span>
                <span className="shrink-0 font-medium" style={{ color: TERM_HEX.cyan }}>
                  {ev.source}
                </span>
                <span className="shrink-0" style={{ color: TERM_HEX.magenta }}>
                  {ev.type}
                </span>
                <span className="min-w-0 break-words whitespace-pre-wrap">
                  <AnsiText text={ev.message} />
                </span>
              </div>
            ))
          )}
        </div>
      </AppleTerminalFrame>
    </div>
  )
}
