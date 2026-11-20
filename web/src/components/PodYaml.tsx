// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { useCallback, useEffect, useMemo, useState } from 'react'
import { Copy, Download, RotateCw } from 'lucide-react'
import { AppleTerminalFrame } from './AppleTerminalFrame'
import { podYaml } from '../api/pods'
import { TERM_HEX, yamlLineSegments } from './terminalTheme'

export default function PodYaml({ namespace, pod }: { namespace: string; pod: string }) {
  const [yaml, setYaml] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const [copied, setCopied] = useState(false)

  const load = useCallback(async () => {
    setLoading(true)
    try {
      setYaml(await podYaml(namespace, pod))
      setError(null)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setLoading(false)
    }
  }, [namespace, pod])

  useEffect(() => {
    void load()
  }, [load])

  const lines = useMemo(() => yaml.replace(/\n$/, '').split('\n'), [yaml])

  const copy = async () => {
    await navigator.clipboard.writeText(yaml)
    setCopied(true)
    window.setTimeout(() => setCopied(false), 1500)
  }

  const download = () => {
    const url = URL.createObjectURL(new Blob([yaml], { type: 'text/yaml' }))
    const a = document.createElement('a')
    a.href = url
    a.download = `${namespace}_${pod}.yaml`
    a.click()
    URL.revokeObjectURL(url)
  }

  const gutter = String(lines.length).length

  return (
    <div data-testid="pod-yaml" className="h-full">
      <AppleTerminalFrame
        title={`yaml · ${namespace}/${pod}`}
        className="zf-terminal-pro flex flex-col h-full"
        bodyClassName="flex-1 min-h-0 overflow-auto px-4 py-3"
        trailing={
          <div className="flex items-center gap-1.5 shrink-0">
            <button type="button" className={`zf-term-btn${copied ? ' is-on' : ''}`} onClick={() => void copy()} disabled={!yaml}>
              <Copy className="w-3.5 h-3.5" />
              <span className="ml-1">{copied ? 'copied' : 'copy'}</span>
            </button>
            <button type="button" className="zf-term-btn" onClick={download} disabled={!yaml} aria-label="Download YAML">
              <Download className="w-3.5 h-3.5" />
            </button>
            <button type="button" className="zf-term-btn" onClick={() => void load()} aria-label="Reload YAML">
              <RotateCw className="w-3.5 h-3.5" />
            </button>
          </div>
        }
      >
        <div className="whitespace-pre" style={{ color: TERM_HEX.fg }}>
          <div className="mb-1">
            <span style={{ color: TERM_HEX.green }}>zorvia@pods</span>
            <span style={{ color: TERM_HEX.dim }}>:</span>
            <span style={{ color: TERM_HEX.blue }}>~</span>
            <span style={{ color: TERM_HEX.dim }}>$ </span>
            kubectl get pod -n {namespace} {pod} -o yaml
          </div>
          {error ? (
            <div style={{ color: TERM_HEX.red }}>error: {error}</div>
          ) : loading && !yaml ? (
            <div style={{ color: TERM_HEX.dim }}>Loading…</div>
          ) : (
            lines.map((line, i) => (
              <div key={i}>
                <span className="select-none" style={{ color: '#4a4a4c' }}>
                  {String(i + 1).padStart(gutter, ' ')}
                  {'  '}
                </span>
                {yamlLineSegments(line).map((s, j) => (
                  <span key={j} style={s.color ? { color: s.color } : undefined}>
                    {s.text}
                  </span>
                ))}
              </div>
            ))
          )}
        </div>
      </AppleTerminalFrame>
    </div>
  )
}
