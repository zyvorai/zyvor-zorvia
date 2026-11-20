// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { useEffect, useRef, useState } from 'react'
import { Terminal as XTerm } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import '@xterm/xterm/css/xterm.css'
import { getToken } from '../api/client'
import { RotateCw } from 'lucide-react'
import { AppleTerminalFrame } from './AppleTerminalFrame'
import { MAC_TERMINAL_OPTIONS } from './terminalTheme'

interface TerminalProps {
  vmName: string
}

type Status = 'connecting' | 'connected' | 'disconnected'

const GREY = '\x1b[90m'
const RESET = '\x1b[0m'

export default function Terminal({ vmName }: TerminalProps) {
  const terminalRef = useRef<HTMLDivElement>(null)
  const [status, setStatus] = useState<Status>('connecting')
  const [connectAttempt, setConnectAttempt] = useState(0)
  const [copyOnSelect, setCopyOnSelect] = useState(true)
  const copyRef = useRef(copyOnSelect)
  useEffect(() => {
    copyRef.current = copyOnSelect
  }, [copyOnSelect])

  useEffect(() => {
    const host = terminalRef.current
    if (!host) return
    setStatus('connecting')
    host.replaceChildren()

    const term = new XTerm({ ...MAC_TERMINAL_OPTIONS, cursorBlink: true })
    const fit = new FitAddon()
    term.loadAddon(fit)
    term.open(host)
    fit.fit()
    const ro = new ResizeObserver(() => fit.fit())
    ro.observe(host)

    const selSub = term.onSelectionChange(() => {
      const sel = term.getSelection()
      if (copyRef.current && sel) void navigator.clipboard?.writeText(sel).catch(() => {})
    })

    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
    const token = getToken()
    const wsUrl = `${protocol}//${window.location.host}/ws/console/${vmName}${token ? `?token=${encodeURIComponent(token)}` : ''}`
    const ws = new WebSocket(wsUrl, 'plain.kubevirt.io')
    ws.binaryType = 'arraybuffer'

    ws.onopen = () => {
      setStatus('connected')
      term.write(`${GREY}Connected to ${vmName} serial console — press Enter for a prompt${RESET}\r\n`)
      term.focus()
    }

    ws.onmessage = (event) => {
      if (event.data instanceof ArrayBuffer) {
        term.write(new Uint8Array(event.data))
      } else {
        term.write(event.data)
      }
    }

    ws.onerror = () => {
      term.write(`\r\n\x1b[31mWebSocket error${RESET}\r\n`)
    }

    ws.onclose = () => {
      setStatus('disconnected')
      term.write(`\r\n${GREY}[connection closed]${RESET}\r\n`)
    }

    term.onData((data) => {
      if (ws.readyState === WebSocket.OPEN) {
        ws.send(data)
      }
    })

    return () => {
      ro.disconnect()
      selSub.dispose()
      ws.close()
      term.dispose()
    }
  }, [vmName, connectAttempt])

  const statusMeta: Record<Status, { label: string; color: string }> = {
    connecting: { label: 'Connecting…', color: '#ffd60a' },
    connected: { label: 'Connected', color: '#28c840' },
    disconnected: { label: 'Disconnected', color: '#8e8e93' },
  }
  const meta = statusMeta[status]

  return (
    <AppleTerminalFrame
      title={`zorvia console ${vmName} — tty`}
      live={status === 'connected'}
      className="zf-terminal-pro"
      trailing={
        <div className="flex items-center gap-1.5 shrink-0">
          {status !== 'connected' && (
            <span className="text-[11px] mr-1" style={{ color: meta.color }}>
              {meta.label}
            </span>
          )}
          <button
            type="button"
            className={`zf-term-btn${copyOnSelect ? ' is-on' : ''}`}
            onClick={() => setCopyOnSelect((v) => !v)}
            aria-pressed={copyOnSelect}
          >
            copy on select
          </button>
          <button
            type="button"
            className="zf-term-btn"
            onClick={() => setConnectAttempt((n) => n + 1)}
            aria-label="Reconnect"
            title="Reconnect"
          >
            <RotateCw className="w-3.5 h-3.5" />
          </button>
        </div>
      }
      bodyClassName="p-2"
    >
      <div ref={terminalRef} className="w-full" style={{ height: '520px' }} data-testid="vm-serial-console" />
    </AppleTerminalFrame>
  )
}
