// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { useState, useEffect, useMemo } from 'react'
import { apiFetch } from '../api/client'
import ErrorBanner from '../components/ErrorBanner'
import { AppleTerminalFrame } from '../components/AppleTerminalFrame'
import { PageHeader, Card, CardBody } from '../components/ui'
import { SkeletonCard } from '../components/Skeleton'
import { formatUserError } from '../utils/apiError'
import { toastFailure } from '../utils/toastError'
import { hintsForError } from '../utils/daemonHints'
import { useToastContext } from '../contexts/ToastContext'

interface DiskImage { name: string; path: string; format: string; size_bytes: number; mod_time?: string }

function formatBytes(bytes: number): string {
  if (!bytes) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  const i = Math.floor(Math.log(bytes) / Math.log(1024))
  return `${(bytes / Math.pow(1024, i)).toFixed(1)} ${units[i]}`
}

/** Terminal.app ANSI-ish palette per image format. */
const FORMAT_COLOR: Record<string, string> = {
  blank: '#eaec23',
  containerdisk: '#14f0f0',
  qcow2: '#f935f8',
  raw: '#ff9f0a',
  iso: '#31e722',
  vmdk: '#5ac8fa',
  vhd: '#5ac8fa',
  vhdx: '#5ac8fa',
}

function formatColor(format: string): string {
  return FORMAT_COLOR[format.toLowerCase()] ?? '#cbcccd'
}

/** Registry refs and URLs read as links; `blank:` sizes stay plain. */
function pathColor(path: string): string {
  if (path.startsWith('blank:')) return '#818383'
  if (/^(https?:\/\/|[\w.-]+\.[a-z]{2,}(:\d+)?\/)/i.test(path)) return '#64d2ff'
  return '#31e722'
}

function splitRef(path: string): [string, string] {
  const i = path.lastIndexOf(':')
  if (i <= 0 || path.startsWith('blank:') || path.slice(i).includes('/')) return [path, '']
  return [path.slice(0, i), path.slice(i)]
}

const PROMPT_USER = '#31e722'
const PROMPT_PATH = '#5ac8fa'
const DIM = '#818383'

export default function DiskImages() {
  const toast = useToastContext()
  const [images, setImages] = useState<DiskImage[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [search, setSearch] = useState('')
  const [selected, setSelected] = useState<Set<string>>(new Set())

  const fetchImages = async () => {
    setLoading(true); setError(null)
    try {
      const res = await apiFetch('/api/images')
      if (!res.ok) throw new Error(`HTTP ${res.status}`)
      const data = await res.json()
      setImages(Array.isArray(data) ? data : data.images || data.disk_images || [])
    } catch (err: unknown) {
      const msg = formatUserError(err)
      setError(msg)
      toastFailure(toast, 'Failed to load disk images', err)
    } finally { setLoading(false) }
  }

  useEffect(() => { fetchImages() }, [])

  const filtered = useMemo(() => {
    if (!search) return images
    const q = search.toLowerCase()
    return images.filter(img => img.name.toLowerCase().includes(q) || img.format.toLowerCase().includes(q) || img.path.toLowerCase().includes(q))
  }, [images, search])

  const totalSize = images.reduce((sum, img) => sum + (img.size_bytes || 0), 0)
  const toggleSelect = (path: string) => setSelected(prev => { const next = new Set(prev); if (next.has(path)) next.delete(path); else next.add(path); return next })
  const allSelected = filtered.length > 0 && filtered.every((img) => selected.has(img.path))
  const toggleAll = () =>
    setSelected((prev) => {
      const next = new Set(prev)
      filtered.forEach((img) => (allSelected ? next.delete(img.path) : next.add(img.path)))
      return next
    })

  const prompt = (
    <>
      <span style={{ color: PROMPT_USER }}>zorvia@images</span>
      <span style={{ color: DIM }}>:</span>
      <span style={{ color: PROMPT_PATH }}>~</span>
      <span style={{ color: DIM }}>$ </span>
    </>
  )

  return (
    <div className="space-y-6">
      <PageHeader
        title="Disk Images"
        description="Browse and manage VM disk images"
        onRefresh={fetchImages}
        refreshing={loading}
      />

      {error && (
        <ErrorBanner
          title="Could not load disk images"
          headline={error}
          hints={hintsForError(error, 'storage')}
          onRetry={fetchImages}
        />
      )}

      <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
        {loading ? (
          Array.from({ length: 4 }).map((_, i) => <SkeletonCard key={i} />)
        ) : (
          <>
            <Card><CardBody className="px-4 py-3"><div className="text-2xl font-bold text-[var(--zf-ink)]">{images.length}</div><div className="text-xs text-[var(--zf-muted)] mt-1">Total Images</div></CardBody></Card>
            <Card><CardBody className="px-4 py-3"><div className="text-2xl font-bold text-[var(--zf-ink)]">{formatBytes(totalSize)}</div><div className="text-xs text-[var(--zf-muted)] mt-1">Total Size</div></CardBody></Card>
            <Card><CardBody className="px-4 py-3"><div className="text-2xl font-bold text-[var(--zf-ink)]">{new Set(images.map(i => i.format)).size}</div><div className="text-xs text-[var(--zf-muted)] mt-1">Formats</div></CardBody></Card>
            <Card><CardBody className="px-4 py-3"><div className="text-2xl font-bold text-[var(--zf-ink)]">{selected.size}</div><div className="text-xs text-[var(--zf-muted)] mt-1">Selected</div></CardBody></Card>
          </>
        )}
      </div>

      <AppleTerminalFrame
        title="images — zorvia — zsh"
        className="zf-terminal-pro"
        bodyClassName="max-h-[70vh] overflow-auto px-4 py-3"
        trailing={
          <div className="flex items-center gap-1.5 shrink-0">
            <input
              type="text"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder="grep…"
              aria-label="Search disk images"
              className="zf-term-btn zf-term-input w-44"
            />
            <button type="button" className={`zf-term-btn${allSelected ? ' is-on' : ''}`} onClick={toggleAll} disabled={filtered.length === 0}>
              {allSelected ? 'none' : 'all'}
            </button>
          </div>
        }
      >
        <div className="whitespace-pre text-[#f2f2f2]" data-testid="disk-images-terminal">
          <div>
            {prompt}
            <span>zorvia images</span>
            {search && (
              <>
                <span style={{ color: DIM }}> | </span>
                <span>grep -i </span>
                <span style={{ color: '#eaec23' }}>{JSON.stringify(search)}</span>
              </>
            )}
          </div>

          {loading ? (
            <div style={{ color: DIM }}>Loading images…<span className="zf-term-caret" /></div>
          ) : filtered.length === 0 ? (
            <div style={{ color: DIM }}>
              {images.length === 0 ? 'total 0 — no disk images found' : 'grep: no images match'}
            </div>
          ) : (
            <>
              <div style={{ color: DIM }}>
                total {filtered.length} · {formatBytes(filtered.reduce((s, i) => s + (i.size_bytes || 0), 0))}
                {selected.size > 0 && ` · ${selected.size} selected`}
              </div>
              <table className="zf-ls mt-1 border-collapse">
                <thead>
                  <tr style={{ color: DIM }}>
                    <th className="w-5" aria-label="Selected" />
                    <th>NAME</th>
                    <th>FORMAT</th>
                    <th className="text-right">SIZE</th>
                    <th>SOURCE</th>
                  </tr>
                </thead>
                <tbody>
                  {filtered.map((img) => {
                    const on = selected.has(img.path)
                    const [ref, tag] = splitRef(img.path)
                    return (
                      <tr
                        key={img.path}
                        className={on ? 'is-on' : undefined}
                        tabIndex={0}
                        aria-selected={on}
                        onClick={() => toggleSelect(img.path)}
                        onKeyDown={(e) => {
                          if (e.key === ' ' || e.key === 'Enter') { e.preventDefault(); toggleSelect(img.path) }
                        }}
                      >
                        <td style={{ color: on ? '#31e722' : '#4a4a4c' }}>{on ? '●' : '○'}</td>
                        <td className="font-semibold text-white">{img.name}</td>
                        <td style={{ color: formatColor(img.format) }}>{img.format}</td>
                        <td className="text-right" style={{ color: img.size_bytes ? '#f935f8' : DIM }}>
                          {img.size_bytes ? formatBytes(img.size_bytes) : '—'}
                        </td>
                        <td title={img.path}>
                          <span style={{ color: pathColor(img.path) }}>{ref}</span>
                          {tag && <span style={{ color: '#eaec23' }}>{tag}</span>}
                        </td>
                      </tr>
                    )
                  })}
                </tbody>
              </table>
            </>
          )}

          {!loading && (
            <div className="mt-1">
              {prompt}
              <span className="zf-term-caret" />
            </div>
          )}
        </div>
      </AppleTerminalFrame>
    </div>
  )
}
