// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

import { apiGet, apiPost, apiPutVoid, apiDelete } from './client'

/** Matches the backend's Role enum (src/api/auth/jwt.rs) exactly --
 * there is no 'operator' role. */
export type UserRole = 'admin' | 'user' | 'viewer'

export interface UserAccount {
  id: string
  username: string
  role: UserRole
  enabled: boolean
  created: string
  last_login?: string | null
}

const API_BASE = '/api/v1'

export async function listUsers(): Promise<UserAccount[]> {
  return apiGet<UserAccount[]>(`${API_BASE}/users`)
}

export async function createUser(username: string, password: string, role: UserRole): Promise<UserAccount> {
  return apiPost<UserAccount>(`${API_BASE}/users`, { username, password, role })
}

export async function deleteUser(id: string): Promise<void> {
  return apiDelete(`${API_BASE}/users/${encodeURIComponent(id)}`)
}

export async function updateUserRole(id: string, role: UserRole): Promise<void> {
  return apiPutVoid(`${API_BASE}/users/${encodeURIComponent(id)}/role`, { role })
}

export async function setUserEnabled(id: string, enabled: boolean): Promise<void> {
  return apiPutVoid(`${API_BASE}/users/${encodeURIComponent(id)}/enabled`, { enabled })
}
