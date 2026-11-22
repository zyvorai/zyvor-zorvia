// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { useCallback, useEffect, useMemo, useState } from 'react'
import { PageHeader, Card, CardBody } from '../components/ui'
import ConfirmDialog from '../components/ConfirmDialog'
import ExperimentalDisabledNotice, { ExperimentalBadge, INPUT_CLASS } from '../components/ExperimentalDisabledNotice'
import { CertificatesTable, CertManagerTable } from '../components/CertificatesTable'
import {
  getCertificateAlerts,
  getCertificateInventory,
  getCertificateStatus,
  listCertManagerCertificates,
  renewCertManagerCertificate,
  type CertificateAlert,
  type CertificateInventory,
  type CertificateStatus,
  type CertManagerCertificate,
  type CertManagerList,
} from '../api/certificates'
import { isExperimentalDisabled } from '../utils/experimental'
import { filterCertificates, renewPhrase, type CertFilter } from '../utils/certificates'
import { formatUserError } from '../utils/apiError'
import { toastFailure } from '../utils/toastError'
import { useToastContext } from '../contexts/ToastContext'
import { usePermissions } from '../hooks/usePermissions'

export default function Certificates() {
  const toast = useToastContext()
  const { canAdmin } = usePermissions()
  const [inventory, setInventory] = useState<CertificateInventory | null>(null)
  const [status, setStatus] = useState<CertificateStatus | null>(null)
  const [alerts, setAlerts] = useState<CertificateAlert[]>([])
  const [cm, setCm] = useState<CertManagerList | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [disabled, setDisabled] = useState<string | null>(null)
  const [filter, setFilter] = useState<CertFilter>('all')
  const [search, setSearch] = useState('')
  const [renewing, setRenewing] = useState<CertManagerCertificate | null>(null)
  const [busy, setBusy] = useState(false)

  const load = useCallback(async () => {
    setLoading(true)
    setError(null)
    try {
      const [inv, st, al, mgr] = await Promise.all([
        getCertificateInventory(),
        getCertificateStatus(),
        getCertificateAlerts(),
        listCertManagerCertificates(),
      ])
      setInventory(inv)
      setStatus(st)
      setAlerts(al)
      setCm(mgr)
      setDisabled(null)
    } catch (e) {
      if (isExperimentalDisabled(e)) setDisabled(e.message)
      else setError(formatUserError(e))
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    load()
  }, [load])

  const shown = useMemo(
    () => filterCertificates(inventory?.certificates ?? [], filter, search),
    [inventory, filter, search],
  )

  const onRenew = async () => {
    const target = renewing
    setRenewing(null)
    if (!target) return
    setBusy(true)
    try {
      await renewCertManagerCertificate(target.namespace, target.name, renewPhrase(target.namespace, target.name))
      toast.success(`Renewal requested for ${target.namespace}/${target.name}`)
      await load()
    } catch (e) {
      if (isExperimentalDisabled(e)) setDisabled(e.message)
      else toastFailure(toast, 'Renew failed', e)
    } finally {
      setBusy(false)
    }
  }

  const header = (
    <PageHeader
      title="Certificates"
      description="Inventory of TLS certificates, expiry alerts and cert-manager renewals."
      eyebrow="Experimental"
      onRefresh={load}
      refreshing={loading}
    />
  )

  if (disabled !== null) {
    return (
      <div className="space-y-6">
        {header}
        <ExperimentalDisabledNotice feature="Certificate lifecycle" message={disabled} />
      </div>
    )
  }

  const s = inventory?.summary
  return (
    <div className="space-y-6">
      {header}
      {!canAdmin && (
        <p className="text-sm text-[var(--zf-warning)]" role="status">
          Certificates need the admin role (cluster.admin). Controls are hidden.
        </p>
      )}
      {error && <p className="text-sm text-[var(--zf-danger)]" role="alert">{error}</p>}

      <Card>
        <CardBody className="space-y-2">
          <div className="text-sm font-medium text-[var(--zf-ink)] flex items-center gap-2">
            Expiry monitoring <ExperimentalBadge />
          </div>
          {status ? (
            <div className="text-sm text-[var(--zf-muted)] space-y-1" data-testid="cert-status">
              <div>Alerts at {status.alert_thresholds_days.join(' / ')} days before expiry, once per threshold. Checked every {Math.round(status.check_interval_secs / 60)} min by the scheduler leader.</div>
              <div>
                Last check: {status.last_check ? new Date(status.last_check).toLocaleString() : 'not yet'} ({status.last_certificates} certificates, {status.last_alerts} alerts)
                {status.last_error && <span className="text-[var(--zf-danger)]"> - last error: {status.last_error}</span>}
              </div>
            </div>
          ) : (
            <p className="text-sm text-[var(--zf-muted)]">{loading ? 'Loading…' : 'No status.'}</p>
          )}
          <p className="text-xs text-[var(--zf-muted)]">
            Only certificate metadata is read; private keys are never read or shown. Unit-tested only; not run against a live cluster or cert-manager.
          </p>
        </CardBody>
      </Card>

      <Card>
        <CardBody className="space-y-3">
          <div className="flex flex-wrap items-center gap-3">
            <div className="text-sm font-medium text-[var(--zf-ink)]">Inventory</div>
            {s && (
              <div className="text-xs text-[var(--zf-muted)]" data-testid="cert-summary">
                {s.total} total, {s.expired} expired, {s.expiring_30d} expiring within 30 days, {s.self_signed} self-signed, {s.weak} weak
              </div>
            )}
          </div>
          <div className="flex flex-wrap gap-2">
            <input aria-label="Search certificates" placeholder="Search name, subject, SAN" value={search} onChange={(e) => setSearch(e.target.value)} className={INPUT_CLASS} />
            <select aria-label="Filter" value={filter} onChange={(e) => setFilter(e.target.value as CertFilter)} className={INPUT_CLASS}>
              <option value="all">All</option>
              <option value="expiring">Expired or expiring within 30 days</option>
              <option value="flagged">Flagged (weak, self-signed, expired)</option>
            </select>
          </div>
          {loading && !inventory ? <p className="text-sm text-[var(--zf-muted)]">Loading…</p> : <CertificatesTable certs={shown} />}
          {inventory && inventory.skipped.length > 0 && (
            <ul className="text-xs text-[var(--zf-warning)] list-disc pl-5" aria-label="Skipped">
              {inventory.skipped.map((m) => <li key={m}>{m}</li>)}
            </ul>
          )}
        </CardBody>
      </Card>

      <Card>
        <CardBody className="space-y-3">
          <div className="text-sm font-medium text-[var(--zf-ink)]">cert-manager</div>
          {cm && !cm.installed ? (
            <p className="text-sm text-[var(--zf-muted)]">cert-manager is not installed in this cluster.</p>
          ) : (
            <CertManagerTable certs={cm?.certificates ?? []} canAdmin={canAdmin} busy={busy} onRenew={setRenewing} />
          )}
        </CardBody>
      </Card>

      <Card>
        <CardBody className="space-y-2">
          <div className="text-sm font-medium text-[var(--zf-ink)]">Recent alerts</div>
          {alerts.length === 0 ? (
            <p className="text-sm text-[var(--zf-muted)]">No alerts.</p>
          ) : (
            <ul className="text-sm space-y-1" aria-label="Certificate alerts">
              {[...alerts].reverse().map((a) => (
                <li key={a.id} className={a.notification_type === 'Error' ? 'text-[var(--zf-danger)]' : a.notification_type === 'Warning' ? 'text-[var(--zf-warning)]' : 'text-[var(--zf-muted)]'}>
                  {new Date(a.created_at).toLocaleString()}: {a.message}
                </li>
              ))}
            </ul>
          )}
        </CardBody>
      </Card>

      {renewing && (
        <ConfirmDialog
          title="Renew certificate?"
          message={`This asks cert-manager to re-issue ${renewing.namespace}/${renewing.name} now (it marks the Certificate as Issuing, like cmctl renew). cert-manager then replaces Secret ${renewing.secret_name ?? ''}; workloads that do not reload certificates may need a restart. The action is audited.`}
          confirmLabel="Renew"
          variant="warning"
          onConfirm={onRenew}
          onCancel={() => setRenewing(null)}
        />
      )}
    </div>
  )
}
