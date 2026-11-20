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

interface SshTerminalProps {
  vmName: string
  user?: string
}

type Status = 'connecting' | 'connected' | 'disconnected'

export default function SshTerminal({ vmName, user = 'zorvia' }: SshTerminalProps) {
  const terminalRef = useRef<HTMLDivElement>(null)
  const [status, setStatus] = useState<Status>('connecting')
  const [connectAttempt, setConnectAttempt] = useState(0)
  const [sshUser, setSshUser] = useState(user)

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

    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
    const token = getToken()
    const params = new URLSearchParams()
    if (token) params.set('token', token)
    params.set('user', sshUser)
    const wsUrl = `${protocol}//${window.location.host}/ws/ssh/${vmName}?${params.toString()}`
    const ws = new WebSocket(wsUrl)
    ws.binaryType = 'arraybuffer'

    ws.onopen = () => {
      setStatus('connected')
      term.write(`\x1b[90mSSH session for ${sshUser}@${vmName}\x1b[0m\r\n`)
      term.focus()
    }
    ws.onmessage = (event) => {
      if (event.data instanceof ArrayBuffer) {
        term.write(new Uint8Array(event.data))
      } else {
        term.write(event.data)
      }
    }
    ws.onerror = () => term.write('\r\n\x1b[31mWebSocket error\x1b[0m\r\n')
    ws.onclose = () => {
      setStatus('disconnected')
      term.write('\r\n\x1b[90m[ssh connection closed]\x1b[0m\r\n')
    }
    term.onData((data) => {
      if (ws.readyState === WebSocket.OPEN) ws.send(data)
    })

    return () => {
      ro.disconnect()
      ws.close()
      term.dispose()
    }
  }, [vmName, sshUser, connectAttempt])

  const statusMeta: Record<Status, { label: string; color: string }> = {
    connecting: { label: 'Connecting…', color: '#ffd60a' },
    connected: { label: 'Connected', color: '#28c840' },
    disconnected: { label: 'Disconnected', color: '#8e8e93' },
  }
  const meta = statusMeta[status]

  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2">
        <label className="text-xs text-[var(--zf-muted)]">User</label>
        <input
          value={sshUser}
          onChange={(e) => setSshUser(e.target.value.replace(/[^A-Za-z0-9._-]/g, ''))}
          className="zf-input zf-input-sm w-40 font-mono text-xs"
          maxLength={32}
        />
      </div>
      <AppleTerminalFrame
        title={`zorvia ssh ${vmName} — ${sshUser}`}
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
        <div ref={terminalRef} className="w-full" style={{ height: '520px' }} />
      </AppleTerminalFrame>
    </div>
  )
}
