// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { useAuth } from '../contexts/AuthContext'

/** The backend's Role enum (src/api/auth/jwt.rs) only ever produces
 * admin/user/viewer -- 'operator' was never a real role and has been
 * removed from here to match. */
export function usePermissions() {
  const { user } = useAuth()
  const role = (user?.role ?? 'viewer').toLowerCase()
  const canWrite = role === 'admin' || role === 'user'
  const canAdmin = role === 'admin'
  const isViewer = role === 'viewer'
  return { role, canWrite, canAdmin, isViewer }
}
