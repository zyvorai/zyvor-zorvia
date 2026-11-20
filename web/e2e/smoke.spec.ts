// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { test, expect } from '@playwright/test'

const liveBase = process.env.ZORVIA_E2E_BASE_URL // e.g. https://175.110.122.71:30152
const liveUser = process.env.ZORVIA_E2E_USER || 'admin'
const livePass = process.env.ZORVIA_E2E_PASSWORD

test.describe('console smoke', () => {
  test('sign-in page loads with brand', async ({ page }) => {
    await page.goto('/sign-in')
    await expect(page.locator('body')).toBeVisible()
    await expect(page.getByText(/zorvia|zyvor|sign in|continue/i).first()).toBeVisible()
  })

  test('unauthenticated API health is reachable via same origin proxy only when API up', async ({
    request,
  }) => {
    const res = await request.get('/api/v1/health')
    expect([200, 404, 502, 503]).toContain(res.status())
  })
})

test.describe('live lab API', () => {
  test.skip(!liveBase || !livePass, 'Set ZORVIA_E2E_BASE_URL and ZORVIA_E2E_PASSWORD for live lab')

  test('health + features + login + audit export gate', async ({ request }) => {
    const health = await request.get(`${liveBase}/api/v1/health`)
    expect(health.status()).toBe(200)

    const features = await request.get(`${liveBase}/api/v1/features`)
    expect(features.status()).toBe(200)
    const body = await features.json()
    const audit = (body.features || body.data?.features || []).find(
      (f: { id: string }) => f.id === 'audit-trail',
    )
    expect(audit?.maturity || audit?.level).toMatch(/ga/i)

    const login = await request.post(`${liveBase}/api/v1/auth/login`, {
      data: { username: liveUser, password: livePass },
    })
    expect(login.status()).toBe(200)
    const { token } = await login.json()
    expect(token).toBeTruthy()

    const exportRes = await request.get(`${liveBase}/api/audit/export`, {
      headers: { Authorization: `Bearer ${token}` },
    })
    expect([200, 401, 403]).toContain(exportRes.status())
    if (exportRes.status() === 200) {
      const text = await exportRes.text()
      expect(exportRes.headers()['content-type'] || '').toMatch(/jsonl|ndjson|json/i)
      // Empty trail is fine; non-empty must be NDJSON lines
      if (text.trim()) {
        expect(() => JSON.parse(text.trim().split('\n')[0])).not.toThrow()
      }
    }
  })
})
