// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { useCallback, useEffect, useRef, useState } from 'react'
import { Terminal as XTerm } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { SearchAddon } from '@xterm/addon-search'
import '@xterm/xterm/css/xterm.css'
import { ChevronDown, ChevronUp, Download, Eraser, Pause, Play, RotateCw } from 'lucide-react'
import { AppleTerminalFrame } from './AppleTerminalFrame'
import { MAC_TERMINAL_OPTIONS, colorizeLogLine } from './terminalTheme'
import { podLogsUrl } from '../api/pods'

const MAX_KEPT_LINES = 20_000
const TAIL_OPTIONS = [100, 500, 2000]

type Status = 'connecting' | 'live' | 'ended' | 'error'

interface PodLogsProps {
  namespace: string
  pod: string
  containers: string[]
  container: string
  onContainerChange: (c: string) => void
}

export default function PodLogs({ namespace, pod, containers, container, onContainerChange }: PodLogsProps) {
  const hostRef = useRef<HTMLDivElement>(null)
  const termRef = useRef<XTerm | null>(null)
  const searchRef = useRef<SearchAddon | null>(null)
  const linesRef = useRef<string[]>([])
  const pendingRef = useRef<string[]>([])
  const followRef = useRef(true)

  const [status, setStatus] = useState<Status>('connecting')
  const [follow, setFollow] = useState(true)
  const [pendingCount, setPendingCount] = useState(0)
  const [tail, setTail] = useState(500)
  const [timestamps, setTimestamps] = useState(true)
  const [previous, setPrevious] = useState(false)
  const [query, setQuery] = useState('')
  const [attempt, setAttempt] = useState(0)

  useEffect(() => {
    followRef.current = follow
    if (follow && pendingRef.current.length && termRef.current) {
      for (const l of pendingRef.current) termRef.current.writeln(colorizeLogLine(l))
      pendingRef.current = []
      setPendingCount(0)
      termRef.current.scrollToBottom()
    }
  }, [follow])

  useEffect(() => {
    const host = hostRef.current
    if (!host) return
    host.replaceChildren()
    linesRef.current = []
    pendingRef.current = []
    setPendingCount(0)
    setStatus('connecting')

    const term = new XTerm({
      ...MAC_TERMINAL_OPTIONS,
      disableStdin: true,
      convertEol: true,
      cursorBlink: false,
      cursorStyle: 'bar',
      cursorInactiveStyle: 'none',
      scrollback: MAX_KEPT_LINES,
    })
    const fit = new FitAddon()
    const search = new SearchAddon()
    term.loadAddon(fit)
    term.loadAddon(search)
    term.open(host)
    fit.fit()
    termRef.current = term
    searchRef.current = search

    const ro = new ResizeObserver(() => {
      try {
        fit.fit()
      } catch {
        // host detached mid-resize
      }
    })
    ro.observe(host)

    term.writeln(`\x1b[90m$ kubectl logs -f ${pod} -n ${namespace} -c ${container}\x1b[0m`)

    const ws = new WebSocket(podLogsUrl(namespace, pod, { container, tail, timestamps, previous }))
    let opened = false
    ws.onopen = () => {
      opened = true
      setStatus('live')
    }
    ws.onmessage = (ev) => {
      if (typeof ev.data !== 'string') return
      const line = ev.data
      const kept = linesRef.current
      kept.push(line)
      if (kept.length > MAX_KEPT_LINES) kept.splice(0, kept.length - MAX_KEPT_LINES)
      if (followRef.current) {
        term.writeln(colorizeLogLine(line))
      } else {
        pendingRef.current.push(line)
        setPendingCount(pendingRef.current.length)
      }
    }
    ws.onerror = () => setStatus('error')
    ws.onclose = () => {
      setStatus((s) => (s === 'error' ? s : 'ended'))
      if (!opened) {
        term.writeln('\x1b[31mconnection failed (requires cluster.admin and a reachable pod)\x1b[0m')
      }
    }

    return () => {
      ws.onclose = null
      ws.close()
      ro.disconnect()
      term.dispose()
      termRef.current = null
      searchRef.current = null
    }
  }, [namespace, pod, container, tail, timestamps, previous, attempt])

  const findNext = useCallback(
    (backwards = false) => {
      const s = searchRef.current
      if (!s || !query) return
      const opts = {
        decorations: {
          matchBackground: '#5a4a00',
          activeMatchBackground: '#ffd60a',
          matchOverviewRuler: '#ffd60a',
          activeMatchColorOverviewRuler: '#ffd60a',
        },
      }
      if (backwards) s.findPrevious(query, opts)
      else s.findNext(query, opts)
    },
    [query],
  )

  const download = () => {
    const blob = new Blob([linesRef.current.join('\n') + '\n'], { type: 'text/plain' })
    const a = document.createElement('a')
    a.href = URL.createObjectURL(blob)
    a.download = `${namespace}_${pod}_${container}.log`
    a.click()
    URL.revokeObjectURL(a.href)
  }

  const trailing = (
    <div className="flex items-center gap-1.5 shrink-0">
      {containers.length > 1 && (
        <select
          aria-label="Container"
          className="zf-term-select"
          value={container}
          onChange={(e) => onContainerChange(e.target.value)}
        >
          {containers.map((c) => (
            <option key={c} value={c}>
              {c}
            </option>
          ))}
        </select>
      )}
      <select
        aria-label="Tail lines"
        className="zf-term-select"
        value={tail}
        onChange={(e) => setTail(Number(e.target.value))}
      >
        {TAIL_OPTIONS.map((n) => (
          <option key={n} value={n}>
            tail {n}
          </option>
        ))}
      </select>
      <button
        type="button"
        className={`zf-term-btn ${timestamps ? 'is-on' : ''}`}
        onClick={() => setTimestamps((v) => !v)}
        title="Show timestamps"
      >
        time
      </button>
      <button
        type="button"
        className={`zf-term-btn ${previous ? 'is-on' : ''}`}
        onClick={() => setPrevious((v) => !v)}
        title="Logs from the previous (crashed) container"
      >
        previous
      </button>
      <input
        aria-label="Find in logs"
        className="zf-term-input"
        placeholder="Find"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter') findNext(e.shiftKey)
        }}
      />
      <button type="button" className="zf-term-btn" onClick={() => findNext(true)} aria-label="Previous match">
        <ChevronUp className="w-3.5 h-3.5" />
      </button>
      <button type="button" className="zf-term-btn" onClick={() => findNext(false)} aria-label="Next match">
        <ChevronDown className="w-3.5 h-3.5" />
      </button>
      <button
        type="button"
        className={`zf-term-btn ${follow ? '' : 'is-on'}`}
        onClick={() => setFollow((v) => !v)}
        aria-label={follow ? 'Pause' : 'Resume'}
        title={follow ? 'Pause' : `Resume${pendingCount ? ` (${pendingCount} new)` : ''}`}
      >
        {follow ? <Pause className="w-3.5 h-3.5" /> : <Play className="w-3.5 h-3.5" />}
        {!follow && pendingCount > 0 && <span className="ml-1">{pendingCount}</span>}
      </button>
      <button type="button" className="zf-term-btn" onClick={() => termRef.current?.clear()} aria-label="Clear">
        <Eraser className="w-3.5 h-3.5" />
      </button>
      <button type="button" className="zf-term-btn" onClick={download} aria-label="Download logs">
        <Download className="w-3.5 h-3.5" />
      </button>
      {(status === 'ended' || status === 'error') && (
        <button type="button" className="zf-term-btn" onClick={() => setAttempt((n) => n + 1)} aria-label="Reconnect">
          <RotateCw className="w-3.5 h-3.5" />
        </button>
      )}
    </div>
  )

  return (
    <AppleTerminalFrame
      title={`logs · ${namespace}/${pod} · ${container}`}
      live={status === 'live' && follow}
      trailing={trailing}
      className="zf-terminal-pro flex flex-col h-full"
      bodyClassName="flex-1 min-h-0 p-2"
    >
      <div ref={hostRef} className="h-full w-full" data-testid="pod-logs" />
    </AppleTerminalFrame>
  )
}
