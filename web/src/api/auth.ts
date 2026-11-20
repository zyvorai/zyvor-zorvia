// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { apiGet } from './client'

export interface LoginRequest {
  username: string
  password: string
  totp_code?: string
}

export interface LoginResponse {
  token: string
  user_id: string
  role: string
  username: string
}

export interface UserInfo {
  id: string
  username: string
  role: string
}

const API_BASE = '/api/v1'

export async function login(req: LoginRequest): Promise<LoginResponse> {
  const res = await fetch(`${API_BASE}/auth/login`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(req),
  })
  if (!res.ok) {
    const body = await res.json().catch(() => null) as {
      error?: { code?: string; message?: string; requires_2fa?: boolean }
    } | null
    if (res.status === 403 && (body?.error?.code === 'requires_2fa' || body?.error?.requires_2fa)) {
      const err = new Error(body?.error?.message || '2FA code required') as Error & {
        requires_2fa?: boolean
      }
      err.requires_2fa = true
      throw err
    }
    if (res.status === 401) throw new Error(body?.error?.message || 'Invalid username or password')
    throw new Error(body?.error?.message || 'Login failed')
  }
  return res.json()
}

export async function getMe(): Promise<UserInfo> {
  return apiGet<UserInfo>(`${API_BASE}/auth/me`)
}

export async function listAuthProviders(): Promise<{ id: string; name: string }[]> {
  return apiGet(`${API_BASE}/auth/providers`)
}

export async function startOidc(providerId: string): Promise<{ url: string }> {
  return apiGet(`${API_BASE}/auth/oidc/${encodeURIComponent(providerId)}`)
}
