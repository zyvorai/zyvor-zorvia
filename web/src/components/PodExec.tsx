// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { useEffect, useRef, useState } from 'react'
import { Terminal as XTerm } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import '@xterm/xterm/css/xterm.css'
import { RotateCw } from 'lucide-react'
import { AppleTerminalFrame } from './AppleTerminalFrame'
import { MAC_TERMINAL_OPTIONS } from './terminalTheme'
import { podExecUrl } from '../api/pods'

type Status = 'connecting' | 'connected' | 'exited' | 'closed'
type Shell = 'auto' | 'bash' | 'sh'

interface PodExecProps {
  namespace: string
  pod: string
  containers: string[]
  container: string
  onContainerChange: (c: string) => void
}

export default function PodExec({ namespace, pod, containers, container, onContainerChange }: PodExecProps) {
  const hostRef = useRef<HTMLDivElement>(null)
  const [status, setStatus] = useState<Status>('connecting')
  const [shell, setShell] = useState<Shell>('auto')
  const [copyOnSelect, setCopyOnSelect] = useState(true)
  const copyRef = useRef(copyOnSelect)
  const [attempt, setAttempt] = useState(0)

  useEffect(() => {
    copyRef.current = copyOnSelect
  }, [copyOnSelect])

  useEffect(() => {
    const host = hostRef.current
    if (!host) return
    host.replaceChildren()
    setStatus('connecting')

    const term = new XTerm({
      ...MAC_TERMINAL_OPTIONS,
      cursorBlink: true,
      cursorStyle: 'block',
      scrollback: 10_000,
    })
    const fit = new FitAddon()
    term.loadAddon(fit)
    term.open(host)
    fit.fit()

    const encoder = new TextEncoder()
    const ws = new WebSocket(podExecUrl(namespace, pod, { container, shell }))
    ws.binaryType = 'arraybuffer'
    let finished = false
    let opened = false

    const sendResize = () => {
      if (ws.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify({ type: 'resize', cols: term.cols, rows: term.rows }))
      }
    }

    ws.onopen = () => {
      opened = true
      setStatus('connected')
      sendResize()
      term.focus()
    }
    ws.onmessage = (ev) => {
      if (ev.data instanceof ArrayBuffer) {
        term.write(new Uint8Array(ev.data))
        return
      }
      const text = String(ev.data)
      try {
        const msg = JSON.parse(text) as { type?: string; code?: number | null; message?: string }
        if (msg.type === 'exit') {
          finished = true
          setStatus('exited')
          term.write(`\r\n\x1b[90m[process exited${msg.code === null || msg.code === undefined ? '' : ` ${msg.code}`}]\x1b[0m\r\n`)
          return
        }
        if (msg.type === 'error') {
          finished = true
          setStatus('closed')
          term.write(`\x1b[31m${msg.message ?? 'exec failed'}\x1b[0m\r\n`)
          return
        }
      } catch {
        // not a control frame
      }
      term.write(text)
    }
    ws.onclose = () => {
      if (!finished) {
        setStatus('closed')
        term.write(
          opened
            ? '\r\n\x1b[90m[connection closed]\x1b[0m\r\n'
            : '\x1b[31mconnection failed (requires cluster.admin and a running container with /bin/sh)\x1b[0m\r\n',
        )
      }
    }

    const dataSub = term.onData((d) => {
      if (ws.readyState === WebSocket.OPEN) ws.send(encoder.encode(d))
    })
    const binSub = term.onBinary((d) => {
      if (ws.readyState !== WebSocket.OPEN) return
      const bytes = new Uint8Array(d.length)
      for (let i = 0; i < d.length; i++) bytes[i] = d.charCodeAt(i) & 0xff
      ws.send(bytes)
    })
    const resizeSub = term.onResize(sendResize)
    const selSub = term.onSelectionChange(() => {
      const sel = term.getSelection()
      if (copyRef.current && sel) void navigator.clipboard?.writeText(sel).catch(() => {})
    })

    const ro = new ResizeObserver(() => {
      try {
        fit.fit()
      } catch {
        // host detached mid-resize
      }
    })
    ro.observe(host)

    return () => {
      ws.onclose = null
      ws.close()
      ro.disconnect()
      dataSub.dispose()
      binSub.dispose()
      resizeSub.dispose()
      selSub.dispose()
      term.dispose()
    }
  }, [namespace, pod, container, shell, attempt])

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
        aria-label="Shell"
        className="zf-term-select"
        value={shell}
        onChange={(e) => setShell(e.target.value as Shell)}
      >
        <option value="auto">auto</option>
        <option value="bash">bash</option>
        <option value="sh">sh</option>
      </select>
      <button
        type="button"
        className={`zf-term-btn ${copyOnSelect ? 'is-on' : ''}`}
        onClick={() => setCopyOnSelect((v) => !v)}
        title="Copy selection to clipboard automatically"
      >
        copy on select
      </button>
      <button type="button" className="zf-term-btn" onClick={() => setAttempt((n) => n + 1)} aria-label="Reconnect">
        <RotateCw className="w-3.5 h-3.5" />
      </button>
    </div>
  )

  const shellLabel = shell === 'auto' ? 'sh' : shell
  return (
    <AppleTerminalFrame
      title={`${shellLabel} · ${namespace}/${pod} · ${container}`}
      live={status === 'connected'}
      trailing={trailing}
      className="zf-terminal-pro flex flex-col h-full"
      bodyClassName="flex-1 min-h-0 p-2"
    >
      <div ref={hostRef} className="h-full w-full" data-testid="pod-exec" />
    </AppleTerminalFrame>
  )
}
