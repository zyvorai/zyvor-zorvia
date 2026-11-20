// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { useState, useEffect, useCallback } from 'react'
import { Users, Plus, Trash2, Loader2, CheckCircle } from 'lucide-react'
import { listUsers, createUser, deleteUser, setUserEnabled, UserAccount, UserRole } from '../api/users'
import { useToastContext } from '../contexts/ToastContext'
import { useConfirm } from '../hooks/useConfirm'
import { useAuth } from '../contexts/AuthContext'
import ConfirmDialog from '../components/ConfirmDialog'
import ErrorBanner from '../components/ErrorBanner'
import { PageHeader, EmptyState, DataTable } from '../components/ui'
import { formatUserError } from '../utils/apiError'
import { toastFailure } from '../utils/toastError'
import { hintsForError } from '../utils/daemonHints'

const ROLES: UserRole[] = ['admin', 'user', 'viewer']

function roleBadge(role: string): string {
  switch (role?.toLowerCase()) {
    case 'admin': return 'text-[var(--zf-danger)] bg-[var(--zf-danger)]/10 border-[var(--zf-danger)]/25'
    case 'user': return 'text-[var(--zf-warning)] bg-[var(--zf-warning)]/10 border-[var(--zf-warning)]/25'
    case 'viewer': return 'text-[var(--zf-success)] bg-[var(--zf-success)]/10 border-[var(--zf-success)]/25'
    default: return 'text-[var(--zf-muted)] bg-[var(--zf-canvas)] border-[var(--zf-hairline)]'
  }
}

const USERNAME_REGEX = /^[a-zA-Z0-9_-]{3,32}$/
const MIN_PASSWORD_LENGTH = 8

export default function AccessControl() {
  const toast = useToastContext()
  const { user: currentUser } = useAuth()
  const { confirmState, confirm, cancel } = useConfirm()
  const [users, setUsers] = useState<UserAccount[]>([])
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [showAdd, setShowAdd] = useState(false)
  const [newUsername, setNewUsername] = useState('')
  const [newPassword, setNewPassword] = useState('')
  const [newRole, setNewRole] = useState<UserRole>('viewer')
  const [adding, setAdding] = useState(false)
  const [addError, setAddError] = useState('')
  const [addSuccess, setAddSuccess] = useState('')
  const [togglingId, setTogglingId] = useState<string | null>(null)

  const fetchUsers = useCallback(async () => {
    setLoading(true)
    setLoadError(null)
    try {
      setUsers(await listUsers())
    } catch (err) {
      const msg = formatUserError(err)
      setLoadError(msg)
      toastFailure(toast, 'Failed to load users', err)
    } finally {
      setLoading(false)
    }
  }, [toast])

  useEffect(() => { fetchUsers() }, [fetchUsers])

  const handleAdd = async () => {
    if (!newUsername.trim()) { setAddError('Username is required'); return }
    if (!USERNAME_REGEX.test(newUsername.trim())) { setAddError('Username must be 3-32 characters: letters, numbers, hyphens, underscores'); return }
    if (!newPassword.trim()) { setAddError('Password is required'); return }
    if (newPassword.length < MIN_PASSWORD_LENGTH) { setAddError(`Password must be at least ${MIN_PASSWORD_LENGTH} characters`); return }
    setAdding(true); setAddError(''); setAddSuccess('')
    try {
      await createUser(newUsername.trim(), newPassword, newRole)
      setAddSuccess(`User "${newUsername}" created`); setNewUsername(''); setNewPassword(''); setShowAdd(false); fetchUsers()
      setTimeout(() => setAddSuccess(''), 3000)
    } catch (err) {
      setAddError(formatUserError(err))
      toastFailure(toast, 'Failed to create user', err)
    } finally { setAdding(false) }
  }

  const handleDelete = async (id: string, username: string) => {
    if (!await confirm('Delete User', `Delete user "${username}"?`, { variant: 'danger', confirmLabel: 'Delete' })) return
    try {
      await deleteUser(id)
      setUsers(prev => prev.filter(u => u.id !== id))
      toast.success(`User "${username}" deleted`)
    } catch (err) {
      toastFailure(toast, 'Failed to delete user', err)
    }
  }

  const handleToggle = async (id: string, enabled: boolean) => {
    setTogglingId(id)
    try {
      await setUserEnabled(id, !enabled)
      setUsers(prev => prev.map(u => u.id === id ? { ...u, enabled: !enabled } : u))
    } catch (err) {
      toastFailure(toast, 'Failed to update user', err)
    } finally {
      setTogglingId(null)
    }
  }

  const adminCount = users.filter(u => u.role === 'admin').length
  const userCount = users.filter(u => u.role === 'user').length
  const viewerCount = users.filter(u => u.role === 'viewer').length

  return (
    <div className="space-y-6">
      <PageHeader
        title="Access Control"
        description="User accounts and role management"
        onRefresh={fetchUsers}
        refreshing={loading}
        primaryAction={
          <button
            type="button"
            onClick={() => setShowAdd(!showAdd)}
            className="zf-btn zf-btn-primary zf-btn-sm"
          >
            <Plus className="w-4 h-4" /> Add User
          </button>
        }
      />

      {loadError && (
        <ErrorBanner
          title="Could not load users"
          headline={loadError}
          hints={hintsForError(loadError, 'auth')}
          onRetry={fetchUsers}
        />
      )}

      {loading && !loadError ? (
        <div className="flex items-center justify-center h-64 text-[var(--zf-muted)]">
          <div className="animate-spin w-6 h-6 border-2 border-[var(--zf-ink)] border-t-transparent rounded-full mr-3" />
          Loading users…
        </div>
      ) : !loadError ? (
        <>

      {addSuccess && <div className="bg-[var(--zf-success)]/10 border border-[var(--zf-success)]/25 rounded-xl px-4 py-3 text-sm text-[var(--zf-success)] flex items-center gap-2"><CheckCircle className="w-4 h-4" />{addSuccess}</div>}

      <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
        <div className="stat-card-blue rounded-xl border border-[var(--zf-hairline)] px-4 py-3 transition-all hover:scale-[1.02]">
          <div className="text-2xl font-bold text-[var(--zf-ink)]">{adminCount}</div>
          <div className="text-xs text-[var(--zf-muted)] mt-1">Admins</div>
        </div>
        <div className="stat-card-cyan rounded-xl border border-[var(--zf-hairline)] px-4 py-3 transition-all hover:scale-[1.02]">
          <div className="text-2xl font-bold text-[var(--zf-ink)]">{userCount}</div>
          <div className="text-xs text-[var(--zf-muted)] mt-1">Users</div>
        </div>
        <div className="stat-card-green rounded-xl border border-[var(--zf-hairline)] px-4 py-3 transition-all hover:scale-[1.02]">
          <div className="text-2xl font-bold text-[var(--zf-ink)]">{viewerCount}</div>
          <div className="text-xs text-[var(--zf-muted)] mt-1">Viewers</div>
        </div>
      </div>

      {showAdd && (
        <div className="bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-xl p-5 space-y-4">
          <h3 className="text-sm font-semibold text-[var(--zf-ink)]">New User</h3>
          <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
            <div><label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Username</label><input type="text" value={newUsername} onChange={(e) => setNewUsername(e.target.value)} placeholder="username" className="input-field text-sm" /></div>
            <div><label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Password</label><input type="password" value={newPassword} onChange={(e) => setNewPassword(e.target.value)} placeholder="password" className="input-field text-sm" /></div>
            <div><label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Role</label>
              <div className="flex gap-2">
                {ROLES.map(r => (
                  <button key={r} onClick={() => setNewRole(r)} className={`zf-chip capitalize ${newRole === r? 'zf-chip-active' : ''}`}>{r}</button>
                ))}
              </div>
            </div>
          </div>
          {addError && <p className="text-sm text-[var(--zf-danger)]">{addError}</p>}
          <div className="flex gap-2">
            <button onClick={handleAdd} disabled={adding} className="zf-btn zf-btn-primary zf-btn-sm">{adding ? <Loader2 className="w-4 h-4 animate-spin" /> : <Plus className="w-4 h-4" />}{adding ? 'Adding...' : 'Add'}</button>
            <button onClick={() => { setShowAdd(false); setAddError('') }} className="zf-btn zf-btn-ghost zf-btn-sm">Cancel</button>
          </div>
        </div>
      )}

      <div className="bg-[var(--zf-surface)] rounded-xl border border-[var(--zf-hairline)] overflow-hidden">
        <DataTable
          columns={[
            {
              key: 'user',
              header: 'User',
              className: 'px-5',
              render: (user) => {
                const isSelf = currentUser?.id === user.id
                return (
                  <div className="flex items-center gap-3">
                    <div className="w-8 h-8 rounded-full bg-[var(--zf-ink)] flex items-center justify-center text-xs font-bold text-[var(--zf-canvas)] uppercase">{user.username.charAt(0)}</div>
                    <span className="text-[var(--zf-ink)] font-medium">{user.username}</span>
                    {isSelf && <span className="text-[10px] text-[var(--zf-muted)]">(you)</span>}
                  </div>
                )
              },
            },
            {
              key: 'role',
              header: 'Role',
              render: (user) => <span className={`px-2.5 py-0.5 rounded-full text-xs font-medium border capitalize ${roleBadge(user.role)}`}>{user.role}</span>,
            },
            {
              key: 'status',
              header: 'Status',
              render: (user) => (
                <button onClick={() => handleToggle(user.id, user.enabled)} disabled={togglingId === user.id} className="flex items-center gap-1.5 disabled:opacity-50" aria-label={`${user.enabled ? 'Disable' : 'Enable'} user ${user.username}`}>
                  <div className={`relative w-8 h-4 rounded-full transition-colors ${user.enabled ? 'bg-[var(--zf-success)]' : 'bg-[var(--zf-hairline)]'}`} role="switch" aria-checked={user.enabled}><div className={`absolute top-0.5 w-3.5 h-3.5 rounded-full bg-white transition-transform ${user.enabled ? 'translate-x-4' : 'translate-x-0.5'}`} /></div>
                  <span className={`text-xs ${user.enabled ? 'text-[var(--zf-success)]' : 'text-[var(--zf-muted)]'}`}>{user.enabled ? 'Active' : 'Disabled'}</span>
                </button>
              ),
            },
            { key: 'created', header: 'Created', render: (user) => <span className="text-xs text-[var(--zf-muted)]">{user.created ? new Date(user.created).toLocaleDateString() : '-'}</span> },
            { key: 'last_login', header: 'Last Login', render: (user) => <span className="text-xs text-[var(--zf-muted)]">{user.last_login ? new Date(user.last_login).toLocaleString() : 'Never'}</span> },
            {
              key: 'actions',
              header: 'Actions',
              className: 'text-right',
              render: (user) => (
                <button onClick={() => handleDelete(user.id, user.username)} className="p-1.5 text-[var(--zf-muted)] hover:text-[var(--zf-danger)] hover:bg-[var(--zf-danger)]/10 rounded-lg transition-colors" title="Delete user"><Trash2 className="w-4 h-4" /></button>
              ),
            },
          ]}
          rows={users}
          getRowKey={(user) => user.id}
          bordered={false}
          emptyState={<EmptyState icon={<Users className="w-10 h-10" />} title="No users configured" />}
        />
      </div>
        </>
      ) : null}

      {confirmState && (
        <ConfirmDialog
          title={confirmState.title}
          message={confirmState.message}
          confirmLabel={confirmState.confirmLabel ?? 'Delete'}
          variant={confirmState.variant ?? 'danger'}
          onConfirm={confirmState.onConfirm}
          onCancel={cancel}
        />
      )}
    </div>
  )
}
