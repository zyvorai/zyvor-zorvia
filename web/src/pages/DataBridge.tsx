// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { useEffect, useState } from 'react'
import { DatabaseZap, Plus, Trash2, Search, Rocket, Play, Square, RotateCcw, CheckCircle2, ArrowRightLeft, Undo2 } from 'lucide-react'
import {
  listDataBridgeSources,
  createDataBridgeSource,
  deleteDataBridgeSource,
  discoverDataBridgeSource,
  listMigrationPlans,
  createMigrationPlan,
  deleteMigrationPlan,
  assessMigrationPlan,
  provisionMigrationEdge,
  fullLoadMigrationPlan,
  startMigrationCdc,
  stopMigrationCdc,
  restartMigrationCdc,
  validateMigrationPlan,
  cutoverMigrationPlan,
  rollbackMigrationPlan,
  listEdgeClusters,
  listCdcStreams,
  listObjectMigrations,
  createObjectMigration,
  deleteObjectMigration,
  startObjectMigration,
  listMigrationValidations,
  listMigrationCutovers,
  pollAtlasJob,
  isAtlasJobTerminal,
  DataBridgeSource,
  MigrationPlan,
  EdgeDbCluster,
  CdcStream,
  ObjectMigration,
  MigrationValidation,
  MigrationCutover,
} from '../api/databridge'
import { getAtlasStatus } from '../api/atlas'
import { useToastContext } from '../contexts/ToastContext'
import { toastFailure } from '../utils/toastError'
import { formatBytes } from '../utils/format'
import { useConfirm } from '../hooks/useConfirm'
import ConfirmDialog from '../components/ConfirmDialog'
import ErrorBanner from '../components/ErrorBanner'
import { PageHeader } from '../components/ui'

const STATE_STYLES: Record<string, string> = {
  registered: 'text-[var(--zf-muted)] bg-[var(--zf-canvas)] border-[var(--zf-hairline)]',
  discovered: 'text-[var(--zf-link)] bg-[var(--zf-link)]/10 border-[var(--zf-link)]/25',
  draft: 'text-[var(--zf-muted)] bg-[var(--zf-canvas)] border-[var(--zf-hairline)]',
  assessed: 'text-[var(--zf-link)] bg-[var(--zf-link)]/10 border-[var(--zf-link)]/25',
  provisioned: 'text-[var(--zf-link)] bg-[var(--zf-link)]/10 border-[var(--zf-link)]/25',
  loaded: 'text-[var(--zf-warning)] bg-[var(--zf-warning)]/10 border-[var(--zf-warning)]/25',
  cdc_streaming: 'text-[var(--zf-warning)] bg-[var(--zf-warning)]/10 border-[var(--zf-warning)]/25',
  validated: 'text-[var(--zf-success)] bg-[var(--zf-success)]/10 border-[var(--zf-success)]/25',
  cutover_complete: 'text-[var(--zf-success)] bg-[var(--zf-success)]/10 border-[var(--zf-success)]/25',
  ready: 'text-[var(--zf-success)] bg-[var(--zf-success)]/10 border-[var(--zf-success)]/25',
  streaming: 'text-[var(--zf-success)] bg-[var(--zf-success)]/10 border-[var(--zf-success)]/25',
  passed: 'text-[var(--zf-success)] bg-[var(--zf-success)]/10 border-[var(--zf-success)]/25',
  complete: 'text-[var(--zf-success)] bg-[var(--zf-success)]/10 border-[var(--zf-success)]/25',
  error: 'text-[var(--zf-danger)] bg-[var(--zf-danger)]/10 border-[var(--zf-danger)]/25',
  failed: 'text-[var(--zf-danger)] bg-[var(--zf-danger)]/10 border-[var(--zf-danger)]/25',
}

function stateBadge(state: string) {
  return STATE_STYLES[state] ?? 'text-[var(--zf-muted)] bg-[var(--zf-canvas)] border-[var(--zf-hairline)]'
}

export default function DataBridge() {
  const toast = useToastContext()
  const { confirmState, confirm, cancel } = useConfirm()
  const [enabled, setEnabled] = useState<boolean | null>(null)
  const [connected, setConnected] = useState(false)
  const [sources, setSources] = useState<DataBridgeSource[]>([])
  const [plans, setPlans] = useState<MigrationPlan[]>([])
  const [edgeClusters, setEdgeClusters] = useState<EdgeDbCluster[]>([])
  const [cdcStreams, setCdcStreams] = useState<CdcStream[]>([])
  const [objectMigrations, setObjectMigrations] = useState<ObjectMigration[]>([])
  const [validations, setValidations] = useState<MigrationValidation[]>([])
  const [cutovers, setCutovers] = useState<MigrationCutover[]>([])
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [busy, setBusy] = useState<string | null>(null)

  const loadAll = async (silent = false) => {
    if (!silent) setLoadError(null)
    try {
      const status = await getAtlasStatus()
      setEnabled(status.enabled)
      setConnected(status.connected)
      if (!status.enabled || !status.connected) {
        return
      }
      const [s, p, ec, cdc, om, v, co] = await Promise.all([
        listDataBridgeSources(),
        listMigrationPlans(),
        listEdgeClusters(),
        listCdcStreams(),
        listObjectMigrations(),
        listMigrationValidations(),
        listMigrationCutovers(),
      ])
      setSources(s)
      setPlans(p)
      setEdgeClusters(ec)
      setCdcStreams(cdc)
      setObjectMigrations(om)
      setValidations(v)
      setCutovers(co)
    } catch (err) {
      if (!silent) {
        setLoadError(err instanceof Error ? err.message : String(err))
        toastFailure(toast, 'Failed to load DataBridge status', err)
      }
    } finally {
      if (!silent) setLoading(false)
    }
  }

  useEffect(() => {
    setLoading(true)
    void loadAll(false)
    const interval = setInterval(() => void loadAll(true), 15000)
    return () => clearInterval(interval)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const runWrite = async (busyKey: string, actionLabel: string, name: string, op: () => Promise<{ job_id: string }>) => {
    setBusy(busyKey)
    try {
      const envelope = await op()
      toast.success(`${actionLabel} requested for '${name}'`)
      await loadAll(true)
      const job = await pollAtlasJob(envelope.job_id)
      if (isAtlasJobTerminal(job)) {
        if (job.state === 'succeeded') {
          toast.success(`${actionLabel} succeeded for '${name}'`)
        } else {
          toast.error(`${actionLabel} failed for '${name}': ${job.error ?? 'unknown error'}`)
        }
      }
      await loadAll(true)
    } catch (e) {
      toastFailure(toast, `Failed to ${actionLabel.toLowerCase()}`, e)
    } finally {
      setBusy(null)
    }
  }

  const runSync = async (busyKey: string, actionLabel: string, name: string, op: () => Promise<unknown>) => {
    setBusy(busyKey)
    try {
      await op()
      toast.success(`${actionLabel} '${name}'`)
      await loadAll(true)
    } catch (e) {
      toastFailure(toast, `Failed to ${actionLabel.toLowerCase()}`, e)
    } finally {
      setBusy(null)
    }
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center h-full">
        <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-[var(--zf-ink)]"></div>
      </div>
    )
  }

  return (
    <div className="space-y-6">
      {loadError && <ErrorBanner title="Could not load DataBridge status" headline={loadError} onRetry={() => void loadAll()} />}
      <PageHeader
        title="DataBridge"
        description="Cloud-to-edge database migration — sources, staged migration plans, and object-store copies"
        icon={DatabaseZap}
        onRefresh={() => void loadAll()}
      />

      {!enabled && (
        <div className="bg-[var(--zf-surface)] rounded-lg border border-[var(--zf-hairline)] p-6">
          <p className="text-sm text-[var(--zf-muted)]">
            DataBridge is part of the optional Atlas storage integration. Set <code>ATLAS_URL</code> (and{' '}
            <code>ATLAS_TOKEN</code>) on the Zorvia server to enable — see{' '}
            <a href="/docs/ATLAS_INTEGRATION.md" className="underline">
              docs/ATLAS_INTEGRATION.md
            </a>
            .
          </p>
        </div>
      )}

      {enabled && !connected && (
        <div className="bg-[var(--zf-surface)] rounded-lg border border-[var(--zf-hairline)] p-6">
          <p className="text-sm text-[var(--zf-warning)] bg-[var(--zf-warning)]/10 border border-[var(--zf-warning)]/25 rounded px-3 py-2">
            Could not reach the configured Atlas control plane.
          </p>
        </div>
      )}

      {enabled && connected && (
        <>
          <div className="bg-[var(--zf-surface)] rounded-lg border border-[var(--zf-hairline)] p-6 space-y-4">
            <h2 className="text-sm font-semibold text-[var(--zf-ink)]">Sources</h2>
            {sources.length === 0 ? (
              <p className="text-sm text-[var(--zf-muted)]">No source databases registered yet.</p>
            ) : (
              <div className="divide-y divide-[var(--zf-hairline)]">
                {sources.map((s) => (
                  <div key={s.id} className="flex items-center justify-between py-2 gap-3">
                    <div>
                      <div className="font-medium text-sm text-[var(--zf-ink)]">{s.name}</div>
                      <div className="text-xs text-[var(--zf-muted)]">
                        {s.kind} · {s.cloud}
                        {s.endpoint ? ` · ${s.endpoint}` : ''}
                      </div>
                    </div>
                    <div className="flex items-center gap-2 shrink-0">
                      <span className={`px-2 py-0.5 rounded-full text-xs font-medium border ${stateBadge(s.state)}`}>{s.state}</span>
                      {s.state !== 'discovered' && (
                        <button
                          type="button"
                          disabled={busy !== null}
                          onClick={() =>
                            void runWrite(`discover-${s.id}`, 'Discover', s.name, () => discoverDataBridgeSource(s.id))
                          }
                          className="zf-btn zf-btn-ghost zf-btn-sm"
                        >
                          <Search className="w-3.5 h-3.5" />
                          Discover
                        </button>
                      )}
                      <button
                        type="button"
                        disabled={busy !== null}
                        onClick={async () => {
                          if (
                            !(await confirm(`Delete source '${s.name}'`, 'Refused while a migration plan still references it.', {
                              variant: 'danger',
                              confirmLabel: 'Delete',
                            }))
                          ) {
                            return
                          }
                          void runSync(`delete-src-${s.id}`, 'Deleted', s.name, () => deleteDataBridgeSource(s.id))
                        }}
                        className="zf-btn zf-btn-danger zf-btn-sm"
                      >
                        <Trash2 className="w-3.5 h-3.5" />
                      </button>
                    </div>
                  </div>
                ))}
              </div>
            )}
            <CreateSourceForm
              disabled={busy !== null}
              onCreate={(req) => void runSync('create-source', 'Registered', req.name, () => createDataBridgeSource(req))}
            />
          </div>

          <div className="bg-[var(--zf-surface)] rounded-lg border border-[var(--zf-hairline)] p-6 space-y-4">
            <h2 className="text-sm font-semibold text-[var(--zf-ink)]">Migration plans</h2>
            {plans.length === 0 ? (
              <p className="text-sm text-[var(--zf-muted)]">No migration plans yet.</p>
            ) : (
              <div className="divide-y divide-[var(--zf-hairline)]">
                {plans.map((p) => (
                  <PlanRow
                    key={p.id}
                    plan={p}
                    busy={busy}
                    onAssess={() => void runWrite(`assess-${p.id}`, 'Assess', p.name, () => assessMigrationPlan(p.id))}
                    onProvision={() => void runWrite(`provision-${p.id}`, 'Provision', p.name, () => provisionMigrationEdge(p.id))}
                    onFullLoad={() => void runWrite(`fullload-${p.id}`, 'Full load', p.name, () => fullLoadMigrationPlan(p.id))}
                    onCdcStart={() => void runWrite(`cdcstart-${p.id}`, 'Start CDC', p.name, () => startMigrationCdc(p.id))}
                    onCdcStop={() => void runWrite(`cdcstop-${p.id}`, 'Stop CDC', p.name, () => stopMigrationCdc(p.id))}
                    onCdcRestart={() => void runWrite(`cdcrestart-${p.id}`, 'Restart CDC', p.name, () => restartMigrationCdc(p.id))}
                    onValidate={() => void runWrite(`validate-${p.id}`, 'Validate', p.name, () => validateMigrationPlan(p.id))}
                    onCutover={async () => {
                      if (
                        !(await confirm(
                          `Cut over '${p.name}'`,
                          'Switches production traffic to the edge database. Atlas requires the plan to be validated, the last validation to have passed, and CDC lag under 10s — it will reject this with a specific error if any of those aren’t met.',
                          { variant: 'danger', confirmLabel: 'Cut over' },
                        ))
                      ) {
                        return
                      }
                      void runWrite(`cutover-${p.id}`, 'Cutover', p.name, () => cutoverMigrationPlan(p.id))
                    }}
                    onRollback={async () => {
                      if (
                        !(await confirm(`Roll back '${p.name}'`, 'Reverts the cutover. Only possible within the rollback window.', {
                          variant: 'danger',
                          confirmLabel: 'Roll back',
                        }))
                      ) {
                        return
                      }
                      void runWrite(`rollback-${p.id}`, 'Rollback', p.name, () => rollbackMigrationPlan(p.id))
                    }}
                    onDelete={async () => {
                      if (
                        !(await confirm(`Delete plan '${p.name}'`, 'Delete this migration plan?', {
                          variant: 'danger',
                          confirmLabel: 'Delete',
                        }))
                      ) {
                        return
                      }
                      void runSync(`delete-plan-${p.id}`, 'Deleted', p.name, () => deleteMigrationPlan(p.id))
                    }}
                  />
                ))}
              </div>
            )}
            <CreatePlanForm
              sources={sources}
              disabled={busy !== null}
              onCreate={(req) => void runSync('create-plan', 'Created', req.name, () => createMigrationPlan(req))}
            />
          </div>

          <div className="bg-[var(--zf-surface)] rounded-lg border border-[var(--zf-hairline)] p-6 space-y-4">
            <h2 className="text-sm font-semibold text-[var(--zf-ink)]">Object-store migrations</h2>
            {objectMigrations.length === 0 ? (
              <p className="text-sm text-[var(--zf-muted)]">No object-store migrations yet.</p>
            ) : (
              <div className="divide-y divide-[var(--zf-hairline)]">
                {objectMigrations.map((m) => (
                  <div key={m.id} className="flex items-center justify-between py-2 gap-3">
                    <div>
                      <div className="font-medium text-sm text-[var(--zf-ink)]">{m.name}</div>
                      <div className="text-xs text-[var(--zf-muted)]">
                        {m.source_bucket} → {m.dest_bucket} · {m.mode}
                        {m.bytes_total > 0 ? ` · ${formatBytes(m.bytes_done)}/${formatBytes(m.bytes_total)}` : ''}
                      </div>
                    </div>
                    <div className="flex items-center gap-2 shrink-0">
                      <span className={`px-2 py-0.5 rounded-full text-xs font-medium border ${stateBadge(m.state)}`}>{m.state}</span>
                      {m.state === 'created' && (
                        <button
                          type="button"
                          disabled={busy !== null}
                          onClick={() => void runWrite(`start-om-${m.id}`, 'Start', m.name, () => startObjectMigration(m.id))}
                          className="zf-btn zf-btn-ghost zf-btn-sm"
                        >
                          <Play className="w-3.5 h-3.5" />
                          Start
                        </button>
                      )}
                      <button
                        type="button"
                        disabled={busy !== null}
                        onClick={async () => {
                          if (
                            !(await confirm(`Delete object migration '${m.name}'`, 'Delete this object-store migration record?', {
                              variant: 'danger',
                              confirmLabel: 'Delete',
                            }))
                          ) {
                            return
                          }
                          void runSync(`delete-om-${m.id}`, 'Deleted', m.name, () => deleteObjectMigration(m.id))
                        }}
                        className="zf-btn zf-btn-danger zf-btn-sm"
                      >
                        <Trash2 className="w-3.5 h-3.5" />
                      </button>
                    </div>
                  </div>
                ))}
              </div>
            )}
            <CreateObjectMigrationForm
              disabled={busy !== null}
              onCreate={(req) => void runSync('create-om', 'Created', req.name, () => createObjectMigration(req))}
            />
          </div>

          <ActivitySection edgeClusters={edgeClusters} cdcStreams={cdcStreams} validations={validations} cutovers={cutovers} />
        </>
      )}

      {confirmState && (
        <ConfirmDialog
          title={confirmState.title}
          message={confirmState.message}
          confirmLabel={confirmState.confirmLabel}
          variant={confirmState.variant}
          onConfirm={confirmState.onConfirm}
          onCancel={cancel}
        />
      )}
    </div>
  )
}

function PlanRow({
  plan,
  busy,
  onAssess,
  onProvision,
  onFullLoad,
  onCdcStart,
  onCdcStop,
  onCdcRestart,
  onValidate,
  onCutover,
  onRollback,
  onDelete,
}: {
  plan: MigrationPlan
  busy: string | null
  onAssess: () => void
  onProvision: () => void
  onFullLoad: () => void
  onCdcStart: () => void
  onCdcStop: () => void
  onCdcRestart: () => void
  onValidate: () => void
  onCutover: () => void
  onRollback: () => void
  onDelete: () => void
}) {
  const disabled = busy !== null
  return (
    <div className="flex items-center justify-between py-2 gap-3 flex-wrap">
      <div>
        <div className="font-medium text-sm text-[var(--zf-ink)]">{plan.name}</div>
        <div className="text-xs text-[var(--zf-muted)]">
          readiness {plan.readiness_score} · source {plan.source_id}
        </div>
      </div>
      <div className="flex items-center gap-2 shrink-0 flex-wrap">
        <span className={`px-2 py-0.5 rounded-full text-xs font-medium border ${stateBadge(plan.state)}`}>{plan.state}</span>
        {(plan.state === 'draft' || plan.state === 'discovered') && (
          <button type="button" disabled={disabled} onClick={onAssess} className="zf-btn zf-btn-ghost zf-btn-sm">
            Assess
          </button>
        )}
        {plan.state === 'assessed' && (
          <button type="button" disabled={disabled} onClick={onProvision} className="zf-btn zf-btn-ghost zf-btn-sm">
            <Rocket className="w-3.5 h-3.5" />
            Provision
          </button>
        )}
        {plan.state === 'provisioned' && (
          <button type="button" disabled={disabled} onClick={onFullLoad} className="zf-btn zf-btn-ghost zf-btn-sm">
            Full load
          </button>
        )}
        {plan.state === 'loaded' && !plan.cdc_stream_id && (
          <button type="button" disabled={disabled} onClick={onCdcStart} className="zf-btn zf-btn-ghost zf-btn-sm">
            <Play className="w-3.5 h-3.5" />
            Start CDC
          </button>
        )}
        {plan.cdc_stream_id && plan.state !== 'cutover_complete' && (
          <>
            <button type="button" disabled={disabled} onClick={onCdcStop} className="zf-btn zf-btn-ghost zf-btn-sm">
              <Square className="w-3.5 h-3.5" />
              Stop CDC
            </button>
            <button type="button" disabled={disabled} onClick={onCdcRestart} className="zf-btn zf-btn-ghost zf-btn-sm">
              <RotateCcw className="w-3.5 h-3.5" />
              Restart CDC
            </button>
          </>
        )}
        {(plan.state === 'loaded' || plan.state === 'cdc_streaming') && (
          <button type="button" disabled={disabled} onClick={onValidate} className="zf-btn zf-btn-ghost zf-btn-sm">
            <CheckCircle2 className="w-3.5 h-3.5" />
            Validate
          </button>
        )}
        {plan.state === 'validated' && (
          <button type="button" disabled={disabled} onClick={onCutover} className="zf-btn zf-btn-danger zf-btn-sm">
            <ArrowRightLeft className="w-3.5 h-3.5" />
            Cutover
          </button>
        )}
        {plan.state === 'cutover_complete' && (
          <button type="button" disabled={disabled} onClick={onRollback} className="zf-btn zf-btn-danger zf-btn-sm">
            <Undo2 className="w-3.5 h-3.5" />
            Rollback
          </button>
        )}
        <button type="button" disabled={disabled} onClick={onDelete} className="zf-btn zf-btn-danger zf-btn-sm">
          <Trash2 className="w-3.5 h-3.5" />
        </button>
      </div>
    </div>
  )
}

function ActivitySection({
  edgeClusters,
  cdcStreams,
  validations,
  cutovers,
}: {
  edgeClusters: EdgeDbCluster[]
  cdcStreams: CdcStream[]
  validations: MigrationValidation[]
  cutovers: MigrationCutover[]
}) {
  if (edgeClusters.length === 0 && cdcStreams.length === 0 && validations.length === 0 && cutovers.length === 0) {
    return null
  }
  return (
    <div className="bg-[var(--zf-surface)] rounded-lg border border-[var(--zf-hairline)] p-6 space-y-4">
      <h2 className="text-sm font-semibold text-[var(--zf-ink)]">Activity</h2>

      {edgeClusters.length > 0 && (
        <div>
          <h3 className="text-xs font-medium text-[var(--zf-muted)] mb-2">Edge DB clusters</h3>
          <div className="divide-y divide-[var(--zf-hairline)]">
            {edgeClusters.map((c) => (
              <div key={c.id} className="flex items-center justify-between py-1.5 text-xs">
                <span className="text-[var(--zf-ink)]">
                  {c.engine} · {c.service_endpoint ?? c.cr_name}
                </span>
                <span className={`px-2 py-0.5 rounded-full font-medium border ${stateBadge(c.state)}`}>{c.state}</span>
              </div>
            ))}
          </div>
        </div>
      )}

      {cdcStreams.length > 0 && (
        <div>
          <h3 className="text-xs font-medium text-[var(--zf-muted)] mb-2">CDC streams</h3>
          <div className="divide-y divide-[var(--zf-hairline)]">
            {cdcStreams.map((s) => (
              <div key={s.id} className="flex items-center justify-between py-1.5 text-xs">
                <span className="text-[var(--zf-ink)]">
                  {s.engine} · lag {s.lag_seconds}s · {s.events_total} events
                </span>
                <span className={`px-2 py-0.5 rounded-full font-medium border ${stateBadge(s.state)}`}>{s.state}</span>
              </div>
            ))}
          </div>
        </div>
      )}

      {validations.length > 0 && (
        <div>
          <h3 className="text-xs font-medium text-[var(--zf-muted)] mb-2">Validations</h3>
          <div className="divide-y divide-[var(--zf-hairline)]">
            {validations.map((v) => (
              <div key={v.id} className="flex items-center justify-between py-1.5 text-xs">
                <span className="text-[var(--zf-ink)]">
                  {v.kind} · {v.tables_total - v.tables_mismatched}/{v.tables_total} tables matched
                </span>
                <span className={`px-2 py-0.5 rounded-full font-medium border ${stateBadge(v.state)}`}>{v.state}</span>
              </div>
            ))}
          </div>
        </div>
      )}

      {cutovers.length > 0 && (
        <div>
          <h3 className="text-xs font-medium text-[var(--zf-muted)] mb-2">Cutovers</h3>
          <div className="divide-y divide-[var(--zf-hairline)]">
            {cutovers.map((c) => (
              <div key={c.id} className="flex items-center justify-between py-1.5 text-xs">
                <span className="text-[var(--zf-ink)]">
                  {c.from_endpoint} → {c.to_endpoint}
                </span>
                <span className={`px-2 py-0.5 rounded-full font-medium border ${stateBadge(c.state)}`}>{c.state}</span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  )
}

function CreateSourceForm({
  disabled,
  onCreate,
}: {
  disabled: boolean
  onCreate: (req: { name: string; kind: 'postgres' | 'mysql' | 'mariadb' | 'oracle' | 'sqlserver' | 'mongodb'; endpoint?: string; port?: number; database?: string }) => void
}) {
  const [name, setName] = useState('')
  const [kind, setKind] = useState<'postgres' | 'mysql' | 'mariadb' | 'oracle' | 'sqlserver' | 'mongodb'>('postgres')
  const [endpoint, setEndpoint] = useState('')
  const [database, setDatabase] = useState('')

  return (
    <div className="flex items-end gap-2 flex-wrap pt-2 border-t border-[var(--zf-hairline)]">
      <div>
        <label className="block text-xs text-[var(--zf-muted)] mb-1">Name</label>
        <input value={name} onChange={(e) => setName(e.target.value)} placeholder="prod-pg" className="w-36 px-2.5 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm" />
      </div>
      <div>
        <label className="block text-xs text-[var(--zf-muted)] mb-1">Engine</label>
        <select value={kind} onChange={(e) => setKind(e.target.value as typeof kind)} className="px-2.5 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm">
          <option value="postgres">PostgreSQL</option>
          <option value="mysql">MySQL</option>
          <option value="mariadb">MariaDB</option>
          <option value="oracle">Oracle</option>
          <option value="sqlserver">SQL Server</option>
          <option value="mongodb">MongoDB</option>
        </select>
      </div>
      <div>
        <label className="block text-xs text-[var(--zf-muted)] mb-1">Endpoint</label>
        <input value={endpoint} onChange={(e) => setEndpoint(e.target.value)} placeholder="prod-pg.rds.amazonaws.com" className="w-56 px-2.5 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm" />
      </div>
      <div>
        <label className="block text-xs text-[var(--zf-muted)] mb-1">Database</label>
        <input value={database} onChange={(e) => setDatabase(e.target.value)} placeholder="appdb" className="w-32 px-2.5 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm" />
      </div>
      <button
        type="button"
        disabled={disabled || !name.trim()}
        onClick={() => {
          onCreate({ name: name.trim(), kind, endpoint: endpoint.trim() || undefined, database: database.trim() || undefined })
          setName('')
          setEndpoint('')
          setDatabase('')
        }}
        className="zf-btn zf-btn-primary zf-btn-sm"
      >
        <Plus className="w-4 h-4" />
        Register source
      </button>
    </div>
  )
}

function CreatePlanForm({
  sources,
  disabled,
  onCreate,
}: {
  sources: DataBridgeSource[]
  disabled: boolean
  onCreate: (req: { name: string; source_id: string }) => void
}) {
  const [name, setName] = useState('')
  const [sourceId, setSourceId] = useState(sources[0]?.id ?? '')

  useEffect(() => {
    if (!sourceId && sources[0]) setSourceId(sources[0].id)
  }, [sources, sourceId])

  if (sources.length === 0) {
    return <p className="text-xs text-[var(--zf-muted)] pt-2 border-t border-[var(--zf-hairline)]">Register a source above before creating a migration plan.</p>
  }

  return (
    <div className="flex items-end gap-2 flex-wrap pt-2 border-t border-[var(--zf-hairline)]">
      <div>
        <label className="block text-xs text-[var(--zf-muted)] mb-1">Plan name</label>
        <input value={name} onChange={(e) => setName(e.target.value)} placeholder="prod-pg-migration" className="w-48 px-2.5 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm" />
      </div>
      <div>
        <label className="block text-xs text-[var(--zf-muted)] mb-1">Source</label>
        <select value={sourceId} onChange={(e) => setSourceId(e.target.value)} className="px-2.5 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm">
          {sources.map((s) => (
            <option key={s.id} value={s.id}>
              {s.name}
            </option>
          ))}
        </select>
      </div>
      <button
        type="button"
        disabled={disabled || !name.trim() || !sourceId}
        onClick={() => {
          onCreate({ name: name.trim(), source_id: sourceId })
          setName('')
        }}
        className="zf-btn zf-btn-primary zf-btn-sm"
      >
        <Plus className="w-4 h-4" />
        Create plan
      </button>
    </div>
  )
}

function CreateObjectMigrationForm({
  disabled,
  onCreate,
}: {
  disabled: boolean
  onCreate: (req: { name: string; source_endpoint: string; source_bucket: string; dest_endpoint: string; dest_bucket: string }) => void
}) {
  const [name, setName] = useState('')
  const [sourceEndpoint, setSourceEndpoint] = useState('')
  const [sourceBucket, setSourceBucket] = useState('')
  const [destEndpoint, setDestEndpoint] = useState('')
  const [destBucket, setDestBucket] = useState('')

  return (
    <div className="flex items-end gap-2 flex-wrap pt-2 border-t border-[var(--zf-hairline)]">
      <div>
        <label className="block text-xs text-[var(--zf-muted)] mb-1">Name</label>
        <input value={name} onChange={(e) => setName(e.target.value)} placeholder="cloud-to-rgw" className="w-36 px-2.5 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm" />
      </div>
      <div>
        <label className="block text-xs text-[var(--zf-muted)] mb-1">Source endpoint</label>
        <input value={sourceEndpoint} onChange={(e) => setSourceEndpoint(e.target.value)} placeholder="s3.amazonaws.com" className="w-44 px-2.5 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm" />
      </div>
      <div>
        <label className="block text-xs text-[var(--zf-muted)] mb-1">Source bucket</label>
        <input value={sourceBucket} onChange={(e) => setSourceBucket(e.target.value)} placeholder="my-cloud-bucket" className="w-36 px-2.5 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm" />
      </div>
      <div>
        <label className="block text-xs text-[var(--zf-muted)] mb-1">Dest endpoint</label>
        <input value={destEndpoint} onChange={(e) => setDestEndpoint(e.target.value)} placeholder="rgw.zorvia.svc:80" className="w-40 px-2.5 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm" />
      </div>
      <div>
        <label className="block text-xs text-[var(--zf-muted)] mb-1">Dest bucket</label>
        <input value={destBucket} onChange={(e) => setDestBucket(e.target.value)} placeholder="edge-bucket" className="w-32 px-2.5 py-1.5 bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-md text-sm" />
      </div>
      <button
        type="button"
        disabled={disabled || !name.trim() || !sourceEndpoint.trim() || !sourceBucket.trim() || !destEndpoint.trim() || !destBucket.trim()}
        onClick={() => {
          onCreate({
            name: name.trim(),
            source_endpoint: sourceEndpoint.trim(),
            source_bucket: sourceBucket.trim(),
            dest_endpoint: destEndpoint.trim(),
            dest_bucket: destBucket.trim(),
          })
          setName('')
          setSourceEndpoint('')
          setSourceBucket('')
          setDestEndpoint('')
          setDestBucket('')
        }}
        className="zf-btn zf-btn-primary zf-btn-sm"
      >
        <Plus className="w-4 h-4" />
        Create object migration
      </button>
    </div>
  )
}
