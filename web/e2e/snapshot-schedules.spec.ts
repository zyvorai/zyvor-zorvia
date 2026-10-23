// Copyright 2026 Zyvor AI Labs · Apache-2.0
import { test, expect } from '@playwright/test'

// The VM's Snapshots tab lists its snapshot schedules, an admin can add one (scoped to
// this VM only) and pause or delete it. The API is mocked, so this runs against the
// preview build with no cluster.
test('Snapshots tab: list, add (this VM only), pause and delete a schedule', async ({ page }) => {
  let schedules = [
    {
      id: 'nightly', name: 'nightly', vm_names: ['web1'], schedule_type: 'daily',
      retention: { max_snapshots: 10, max_age_days: 30, keep_last_n: 5 },
      enabled: true, last_run: null, next_run: new Date(Date.now() + 3600_000).toISOString(),
      last_error: 'web1: boom',
    },
  ]
  let created: Record<string, unknown> | null = null
  const calls: string[] = []

  await page.addInitScript(() => localStorage.setItem('zorvia_token', 'test'))
  await page.route('**/api/**', async (route) => {
    const req = route.request()
    const path = new URL(req.url()).pathname
    const json = (status: number, body: unknown) =>
      route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) })

    if (path.includes('auth/me')) return json(200, { id: '1', role: 'admin', username: 'admin' })
    if (path === '/api/vms/web1') return json(200, { name: 'web1', namespace: 'default', state: 'running', cpu: 1, memory: '1Gi' })
    if (path === '/api/vms/web1/snapshot-schedules') return json(200, schedules)
    if (path === '/api/snapshot-schedules' && req.method() === 'POST') {
      created = req.postDataJSON()
      schedules = [...schedules, { ...schedules[0], id: String(created!.name), name: String(created!.name), last_error: null }]
      return json(201, schedules[schedules.length - 1])
    }
    if (path.startsWith('/api/snapshot-schedules/')) {
      calls.push(`${req.method()} ${path}`)
      if (req.method() === 'DELETE') schedules = schedules.filter((s) => `/api/snapshot-schedules/${s.name}` !== path)
      return route.fulfill({ status: req.method() === 'DELETE' ? 204 : 200, body: '' })
    }
    const isList = /\/(vms|images|templates|nodes|networks|devices|zones|snapshots|backups|catalog|pods|alerts)\/?$/.test(path)
    return json(200, isList ? [] : {})
  })

  await page.goto('/app/vms/web1')
  await page.getByRole('button', { name: /^snapshots$/i }).click()

  const panel = page.getByRole('region', { name: /scheduled snapshots/i })
  await expect(panel.getByText('nightly')).toBeVisible()
  await expect(panel.getByText(/keep last 5/)).toBeVisible()
  await expect(panel.getByText(/Last run failed: web1: boom/)).toBeVisible()

  await panel.getByRole('button', { name: /add schedule/i }).click()
  await panel.getByLabel('Name').fill('Bad Name')
  await expect(panel.getByText(/lowercase letters/i)).toBeVisible()
  await expect(panel.getByRole('button', { name: /^create$/i })).toBeDisabled()
  await panel.getByLabel('Name').fill('weekly-keep')
  await panel.getByLabel('Frequency').selectOption('weekly')
  await panel.getByRole('button', { name: /^create$/i }).click()
  await expect.poll(() => created).not.toBeNull()
  expect(created).toMatchObject({ name: 'weekly-keep', schedule_type: 'weekly', vm_names: ['web1'], weekday: 'sunday' })

  await panel.getByRole('button', { name: /^pause$/i }).first().click()
  await expect.poll(() => calls).toContain('POST /api/snapshot-schedules/nightly/disable')

  await panel.getByRole('button', { name: /^delete$/i }).first().click()
  await page.getByRole('button', { name: /^delete$/i }).last().click()
  await expect.poll(() => calls).toContain('DELETE /api/snapshot-schedules/nightly')
})
