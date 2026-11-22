// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { useState, useCallback, useRef } from 'react'
import { Upload, Download, Eye, Send, CheckCircle, XCircle, Clock, Loader2, FileText } from 'lucide-react'
import { createVM } from '../api/vm'
import ErrorBanner from '../components/ErrorBanner'
import { PageHeader, DataTable, type DataTableColumn } from '../components/ui'
import { TerminalTextarea } from '../components/AppleTerminalFrame'
import { formatUserError } from '../utils/apiError'

interface VMEntry { name: string; cpus: number; memoryMiB: number; memoryLabel: string; image: string }
type VMStatus = 'pending' | 'submitting' | 'submitted' | 'error'
interface VMImportItem extends VMEntry { status: VMStatus; error?: string }

const exampleYAML = `vms:
  - name: web-01
    cpus: 2
    memory: 2Gi
    image: quay.io/containerdisks/fedora:latest
  - name: db-01
    cpus: 4
    memory: 8Gi
    image: quay.io/containerdisks/fedora:latest
  - name: app-01
    cpus: 2
    memory: 4Gi
    image: quay.io/containerdisks/fedora:latest`

/** Accepts "2Gi"/"2G" (binary/decimal gibi), "512Mi"/"512M", or a bare
 * number (assumed MiB, matching CreateVMRequest.memory's unit). */
function parseMemoryToMiB(raw: string): number | null {
  const m = raw.trim().match(/^(\d+(?:\.\d+)?)\s*(Gi|G|Mi|M)?$/i)
  if (!m) return null
  const value = parseFloat(m[1])
  const unit = (m[2] || '').toLowerCase()
  if (unit === 'gi' || unit === 'g') return Math.round(value * 1024)
  if (unit === 'mi' || unit === 'm') return Math.round(value)
  return Math.round(value)
}

function parseInput(text: string): { entries: VMEntry[]; errors: string[] } {
  const trimmed = text.trim()
  const errors: string[] = []
  const raw: { name?: string; cpus?: number; memory?: string; image?: string }[] = []

  try {
    const parsed = JSON.parse(trimmed)
    const list = parsed.vms || parsed
    if (Array.isArray(list)) {
      raw.push(...list)
    }
  } catch {
    // Not JSON -- fall back to the minimal YAML-subset parser below.
    let current: { name?: string; cpus?: number; memory?: string; image?: string } | null = null
    for (const line of trimmed.split('\n')) {
      const s = line.trim()
      if (s.startsWith('- name:')) {
        if (current) raw.push(current)
        current = { name: s.replace('- name:', '').trim() }
      } else if (s.startsWith('cpus:') && current) {
        current.cpus = parseInt(s.replace('cpus:', '').trim(), 10)
      } else if (s.startsWith('memory:') && current) {
        current.memory = s.replace('memory:', '').trim()
      } else if (s.startsWith('image:') && current) {
        current.image = s.replace('image:', '').trim()
      }
    }
    if (current) raw.push(current)
  }

  const entries: VMEntry[] = []
  raw.forEach((v, i) => {
    const label = v.name ? `"${v.name}"` : `entry #${i + 1}`
    if (!v.name) { errors.push(`${label}: missing name`); return }
    if (!v.image) { errors.push(`${label}: missing image`); return }
    const memoryLabel = v.memory || '2Gi'
    const memoryMiB = parseMemoryToMiB(memoryLabel)
    if (memoryMiB === null) { errors.push(`${label}: invalid memory "${memoryLabel}" (use e.g. 2Gi, 512Mi, or a bare MiB number)`); return }
    entries.push({ name: v.name, cpus: v.cpus && v.cpus > 0 ? v.cpus : 2, memoryMiB, memoryLabel, image: v.image })
  })
  return { entries, errors }
}

export default function BatchImport() {
  const [inputText, setInputText] = useState('')
  const [items, setItems] = useState<VMImportItem[]>([])
  const [previewing, setPreviewing] = useState(false)
  const [submitting, setSubmitting] = useState(false)
  const [dragOver, setDragOver] = useState(false)
  const [parseError, setParseError] = useState('')
  const fileRef = useRef<HTMLInputElement>(null)

  const handlePreview = useCallback(() => {
    setParseError('')
    const { entries, errors } = parseInput(inputText)
    if (entries.length === 0 && errors.length === 0) { setParseError('No VMs found. Check your YAML/JSON format.'); return }
    if (errors.length > 0) { setParseError(errors.join('; ')); return }
    setItems(entries.map((e) => ({ ...e, status: 'pending' })))
    setPreviewing(true)
  }, [inputText])

  const handleSubmitAll = useCallback(async () => {
    setSubmitting(true)
    for (let i = 0; i < items.length; i++) {
      setItems((prev) => prev.map((it, idx) => idx === i ? { ...it, status: 'submitting' } : it))
      try {
        await createVM({ name: items[i].name, cpus: items[i].cpus, memory: items[i].memoryMiB, image: items[i].image })
        setItems((prev) => prev.map((it, idx) => (idx === i ? { ...it, status: 'submitted' } : it)))
      } catch (err) {
        const msg = formatUserError(err)
        setItems((prev) => prev.map((it, idx) => (idx === i ? { ...it, status: 'error', error: msg } : it)))
      }
    }
    setSubmitting(false)
  }, [items])

  const handleFileDrop = useCallback((e: React.DragEvent) => { e.preventDefault(); setDragOver(false); const file = e.dataTransfer.files[0]; if (file) { const reader = new FileReader(); reader.onload = (ev) => setInputText(ev.target?.result as string); reader.readAsText(file) } }, [])
  const handleFileSelect = useCallback((e: React.ChangeEvent<HTMLInputElement>) => { const file = e.target.files?.[0]; if (file) { const reader = new FileReader(); reader.onload = (ev) => setInputText(ev.target?.result as string); reader.readAsText(file) } }, [])
  const handleDownloadTemplate = useCallback(() => { const blob = new Blob([exampleYAML], { type: 'text/yaml' }); const url = URL.createObjectURL(blob); const a = document.createElement('a'); a.href = url; a.download = 'batch-import-template.yaml'; a.click(); URL.revokeObjectURL(url) }, [])

  const statusIcon = (status: VMStatus) => { switch (status) { case 'pending': return <Clock className="w-4 h-4 text-[var(--zf-muted)]" />; case 'submitting': return <Loader2 className="w-4 h-4 text-[var(--zf-link)] animate-spin" />; case 'submitted': return <CheckCircle className="w-4 h-4 text-[var(--zf-success)]" />; case 'error': return <XCircle className="w-4 h-4 text-[var(--zf-danger)]" /> } }
  const submitted = items.filter((i) => i.status === 'submitted').length
  const errors = items.filter((i) => i.status === 'error').length

  const importColumns: DataTableColumn<VMImportItem>[] = [
    {
      key: 'status',
      header: 'Status',
      render: (item) => (
        <div className="flex items-center gap-2">
          {statusIcon(item.status)}
          <span className="text-xs text-[var(--zf-muted)] capitalize">{item.status}</span>
        </div>
      ),
    },
    { key: 'name', header: 'VM Name', render: (item) => <span className="text-[var(--zf-ink)] font-medium">{item.name}</span> },
    { key: 'cpus', header: 'CPUs', render: (item) => <span className="text-[var(--zf-muted)]">{item.cpus}</span> },
    { key: 'memory', header: 'Memory', render: (item) => <span className="text-[var(--zf-muted)]">{item.memoryLabel}</span> },
    {
      key: 'image',
      header: 'Image',
      render: (item) => (
        <span className="text-[var(--zf-muted)] font-mono text-xs">
          {item.image}
          {item.error && <p className="text-[var(--zf-danger)] mt-1">{item.error}</p>}
        </span>
      ),
    },
  ]

  return (
    <div className="w-full space-y-6">
      <PageHeader
        title="Batch Import"
        description="Import multiple VMs from YAML or JSON"
        actions={
          <button onClick={handleDownloadTemplate} className="zf-btn zf-btn-ghost">
            <Download className="w-4 h-4" /> Download Template
          </button>
        }
      />

      {!previewing ? (
        <div className="space-y-4">
          <div onDragOver={(e) => { e.preventDefault(); setDragOver(true) }} onDragLeave={() => setDragOver(false)} onDrop={handleFileDrop} onClick={() => fileRef.current?.click()}
            className={`border-2 border-dashed rounded-xl p-8 text-center cursor-pointer transition-colors ${dragOver ? 'border-[var(--zf-ink)] bg-black/[0.04]' : 'border-[var(--zf-hairline)] hover:border-[var(--zf-muted)] bg-[var(--zf-surface)]'}`}>
            <Upload className="w-8 h-8 text-[var(--zf-muted)] mx-auto mb-3" />
            <p className="text-sm text-[var(--zf-muted)]">Drag &amp; drop a <span className="text-[var(--zf-ink)] font-medium">.yaml</span> or <span className="text-[var(--zf-ink)] font-medium">.json</span> file, or click to browse</p>
            <input ref={fileRef} type="file" accept=".yaml,.yml,.json" className="hidden" onChange={handleFileSelect} />
          </div>
          <div>
            <label className="block text-sm font-medium text-[var(--zf-muted)] mb-2">Or paste YAML/JSON directly</label>
            <TerminalTextarea
              title="batch-import — YAML/JSON"
              value={inputText}
              onChange={(e) => setInputText(e.target.value)}
              rows={12}
              placeholder={exampleYAML}
            />
          </div>
          {parseError && (
            <ErrorBanner title="Could not parse input" headline={parseError} />
          )}
          <button onClick={handlePreview} disabled={!inputText.trim()} className="zf-btn zf-btn-primary"><Eye className="w-4 h-4" /> Preview</button>
        </div>
      ) : (
        <div className="space-y-4">
          <div className="flex items-center gap-4 p-3 bg-[var(--zf-canvas)] border border-[var(--zf-hairline)] rounded-lg text-sm">
            <span className="text-[var(--zf-muted)]"><FileText className="w-4 h-4 inline mr-1" />{items.length} VMs</span>
            {submitted > 0 && <span className="text-[var(--zf-success)]"><CheckCircle className="w-4 h-4 inline mr-1" />{submitted} submitted</span>}
            {errors > 0 && <span className="text-[var(--zf-danger)]"><XCircle className="w-4 h-4 inline mr-1" />{errors} failed</span>}
          </div>
          <div className="bg-[var(--zf-canvas)] border border-[var(--zf-hairline)] rounded-xl overflow-hidden">
            <DataTable columns={importColumns} rows={items} getRowKey={(item) => item.name} bordered={false} />
          </div>
          <div className="flex items-center gap-3">
            <button onClick={handleSubmitAll} disabled={submitting || items.every((i) => i.status === 'submitted')} className="zf-btn zf-btn-primary">{submitting ? <Loader2 className="w-4 h-4 animate-spin" /> : <Send className="w-4 h-4" />}{submitting ? 'Submitting...' : 'Submit All'}</button>
            <button onClick={() => { setPreviewing(false); setItems([]) }} disabled={submitting} className="zf-btn zf-btn-ghost">Back to Editor</button>
          </div>
        </div>
      )}
    </div>
  )
}
