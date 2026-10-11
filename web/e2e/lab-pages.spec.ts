// Copyright 2026 Zyvor AI Labs · Apache-2.0
import { test, expect, type Page } from '@playwright/test'

const liveBase = process.env.ZORVIA_E2E_BASE_URL || process.env.PLAYWRIGHT_BASE_URL
const liveUser = process.env.ZORVIA_E2E_USER || 'admin'
const livePass = process.env.ZORVIA_E2E_PASSWORD

const CONSOLE_PATHS = [
  '/app',
  '/app/vms',
  '/app/create',
  '/app/favorites',
  '/app/templates',
  '/app/compare',
  '/app/batch-import',
  '/app/windows',
  '/app/volumes',
  '/app/storage',
  '/app/disk-images',
  '/app/snapshots',
  '/app/backups',
  '/app/backup-scheduler',
  '/app/network-policies',
  '/app/service-map',
  '/app/zones',
  '/app/migrations',
  '/app/migrations/readiness',
  '/app/health-check',
  '/app/ha-policy',
  '/app/placement',
  '/app/warm-pools',
  '/app/events',
  '/app/schedules',
  '/app/alerts',
  '/app/webhooks',
  '/app/access-control',
  '/app/compliance',
  '/app/security',
  '/app/capacity',
  '/app/analytics',
  '/app/optimizer',
  '/app/cost-estimator',
  '/app/quotas',
  '/app/pods',
  '/app/fleet',
  '/app/vmware-imports',
  '/app/databridge',
  '/app/devices',
]

// Legacy top-level paths must redirect into /app, and unknown paths must 404 cleanly.
const REDIRECTS: Record<string, RegExp> = {
  '/vms': /\/app\/vms$/,
  '/create': /\/app\/create$/,
  '/favorites': /\/app\/favorites$/,
  '/snapshots': /\/app\/snapshots$/,
  '/login': /\/sign-in$/,
}

const MARKETING_PATHS = ['/', '/product', '/platform', '/security', '/sign-in']

async function login(page: Page) {
  await page.goto('/sign-in')
  // Two-step Apple-style identify → password
  const userField = page.locator('#username, input[name="username"], input[placeholder="admin"]').first()
  if (await userField.isVisible().catch(() => false)) {
    await userField.fill(liveUser)
    const cont = page.getByRole('button', { name: /continue/i })
    if (await cont.isVisible().catch(() => false)) await cont.click()
  }
  await page.locator('#password, input[name="password"], input[type="password"]').first().fill(livePass!)
  await page.getByRole('button', { name: /sign in|log in|continue/i }).last().click()
  await expect(page).toHaveURL(/\/app/, { timeout: 30_000 })
}

async function assertPageOk(page: Page, path: string) {
  await page.goto(path, { waitUntil: 'domcontentloaded' })
  await expect(page.locator('body')).toBeVisible()
  // Sticky load failures surface as ErrorBanner copy
  const errorBanner = page.getByText(/could not load|failed to load/i).first()
  if (await errorBanner.isVisible().catch(() => false)) {
    throw new Error(`Error banner on ${path}: ${await errorBanner.textContent()}`)
  }
  // Prefer a heading; fall back to main content
  const heading = page.locator('h1, [class*="PageHeader"], main').first()
  await expect(heading).toBeVisible({ timeout: 20_000 })
}

test.describe('lab console crawl', () => {
  test.skip(!livePass, 'Set ZORVIA_E2E_PASSWORD for live lab crawl')

  test.beforeEach(async ({ page }) => {
    await login(page)
  })

  test('top mega-nav present, no sidebar', async ({ page }) => {
    await page.goto('/app')
    await expect(page.getByRole('navigation', { name: /console/i })).toBeVisible()
    await expect(page.getByRole('link', { name: 'Dashboard' }).first()).toBeVisible()
    await expect(page.getByRole('link', { name: 'Virtual Machines' }).first()).toBeVisible()
    for (const label of ['Compute', 'Storage', 'Ops', 'Automation']) {
      await expect(page.getByRole('button', { name: label }).first()).toBeVisible()
    }
    await expect(page.locator('.console-sidebar')).toHaveCount(0)
  })

  test('all console pages load without sticky errors', async ({ page }) => {
    test.setTimeout(180_000)
    const failures: string[] = []
    for (const path of CONSOLE_PATHS) {
      try {
        await assertPageOk(page, path)
      } catch (e) {
        failures.push(`${path}: ${e instanceof Error ? e.message : String(e)}`)
      }
    }
    expect(failures, failures.join('\n')).toEqual([])
  })

  test('legacy paths redirect and unknown path 404s', async ({ page }) => {
    for (const [from, to] of Object.entries(REDIRECTS)) {
      await page.goto(from, { waitUntil: 'domcontentloaded' })
      await expect(page, `${from} should redirect`).toHaveURL(to, { timeout: 15_000 })
    }
    await page.goto('/definitely-not-a-page', { waitUntil: 'domcontentloaded' })
    await expect(page.locator('body')).toContainText(/not found|404/i)
  })

  test('core functionality spot-checks', async ({ page, request }) => {
    // Dashboard
    await page.goto('/app')
    await expect(page.locator('main')).toBeVisible()

    // VM list
    await page.goto('/app/vms')
    await expect(page.locator('main')).toBeVisible()

    // Open first VM if present
    const vmLink = page.locator('a[href^="/app/vms/"]').first()
    if (await vmLink.count()) {
      await vmLink.click()
      await expect(page).toHaveURL(/\/app\/vms\//)
      await expect(page.locator('main')).toBeVisible()
    }

    // Create wizard (no submit)
    await page.goto('/app/create')
    await expect(page.locator('main')).toBeVisible()

    // Event stream
    await page.goto('/app/events')
    await expect(page.locator('main')).toBeVisible()

    // Access control (admin)
    await page.goto('/app/access-control')
    await expect(page.locator('main')).toBeVisible()

    // API health + login already proven; list VMs with cookie/token via UI session
    const health = await request.get(`${liveBase}/api/v1/health`)
    expect(health.status()).toBe(200)
  })

  test('top nav links to pods', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 1000 })
    await page.goto('/app')
    await page.getByRole('navigation', { name: 'Console' }).getByRole('link', { name: 'Pods', exact: true }).click()
    await expect(page).toHaveURL(/\/app\/pods/)
    await expect(page.getByRole('heading', { name: 'Pods' })).toBeVisible()
    await expect(page.locator('tbody tr').first()).toBeVisible({ timeout: 20_000 })
    await page.screenshot({ path: 'test-results/pods-page.png' })
  })

  test('disk images: terminal listing', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 1000 })
    await page.goto('/app/disk-images')
    const term = page.getByTestId('disk-images-terminal')
    await expect(term).toContainText('zorvia images')
    await expect(term.locator('tbody tr').first()).toBeVisible({ timeout: 20_000 })
    await page.getByLabel('Search disk images').fill('ubuntu')
    await expect(term).toContainText('grep -i "ubuntu"')
    await page.getByLabel('Search disk images').fill('')
    await term.locator('tbody tr').first().click()
    await expect(term.locator('tbody tr.is-on')).toHaveCount(1)
    await page.screenshot({ path: 'test-results/disk-images.png', fullPage: true })
  })

  test('pods: list, logs and exec', async ({ page }) => {
    test.setTimeout(120_000)
    await page.setViewportSize({ width: 1440, height: 1000 })
    await page.goto('/app/pods')
    await expect(page.getByRole('heading', { name: 'Pods' })).toBeVisible()
    await page.getByLabel('Namespace').selectOption('zorvia-system')
    const apiRow = page.locator('tr', { hasText: 'zorvia-api' }).filter({ hasText: 'Running' }).first()
    await expect(apiRow).toBeVisible({ timeout: 20_000 })

    await apiRow.getByRole('button', { name: 'Logs' }).click()
    const logs = page.getByTestId('pod-logs')
    await expect(logs.locator('.xterm-rows')).toContainText(/\S+.*\S+/, { timeout: 20_000 })
    await expect.poll(async () => (await logs.locator('.xterm-rows > div').allTextContents()).filter((l) => l.trim()).length, {
      timeout: 20_000,
    }).toBeGreaterThan(1)
    await page.screenshot({ path: 'test-results/pods-logs.png' })

    await page.getByTestId('pod-panel').getByRole('button', { name: 'Terminal' }).click()
    const exec = page.getByTestId('pod-exec')
    await expect(page.getByTestId('pod-panel').locator('.zf-terminal-live')).toBeVisible({ timeout: 20_000 })
    await exec.click()
    await page.keyboard.type('echo zorvia-$((40+2))-ok\n')
    await expect(exec.locator('.xterm-rows')).toContainText('zorvia-42-ok', { timeout: 15_000 })
    await page.screenshot({ path: 'test-results/pods-exec.png' })
  })

  test('pods: events, yaml, logs tab, delete confirm', async ({ page, context }) => {
    test.setTimeout(120_000)
    await page.setViewportSize({ width: 1440, height: 1000 })
    await page.goto('/app/pods')
    await page.getByLabel('Namespace').selectOption('zorvia-system')
    const apiRow = page.locator('tr', { hasText: 'zorvia-api' }).filter({ hasText: 'Running' }).first()
    await expect(apiRow).toBeVisible({ timeout: 20_000 })
    await apiRow.getByRole('button', { name: 'Logs' }).click()
    const panel = page.getByTestId('pod-panel')

    await panel.getByRole('button', { name: 'Events' }).click()
    await expect(page.getByTestId('pod-events')).toContainText('kubectl events -n zorvia-system', { timeout: 15_000 })
    await expect(page.getByTestId('pod-events')).toContainText(/\d+ events|No events/, { timeout: 15_000 })
    await page.screenshot({ path: 'test-results/pods-events.png' })

    await panel.getByRole('button', { name: 'YAML' }).click()
    const yaml = page.getByTestId('pod-yaml')
    await expect(yaml).toContainText('kind: Pod', { timeout: 15_000 })
    await expect(yaml).toContainText('namespace: zorvia-system')
    await expect(yaml).not.toContainText('managedFields')
    await page.screenshot({ path: 'test-results/pods-yaml.png' })

    const [tab] = await Promise.all([
      context.waitForEvent('page'),
      panel.getByRole('link', { name: 'Open logs in new tab' }).click(),
    ])
    await tab.waitForLoadState('load')
    await expect(tab).toHaveURL(/\/app\/pods\/zorvia-system\/zorvia-api-[^/]+\/logs/)
    await expect(tab.getByTestId('pod-logs-page').locator('.xterm-rows')).toContainText(/\S+/, { timeout: 20_000 })
    await tab.close()

    await apiRow.getByRole('button', { name: /^Delete zorvia-api/ }).click()
    const dialog = page.getByRole('alertdialog')
    await expect(dialog).toContainText('ReplicaSet will create a replacement')
    await dialog.getByRole('button', { name: 'Cancel' }).click()
    await expect(apiRow).toBeVisible()
  })

  test('pods: missing RBAC disables actions with a notice', async ({ page }) => {
    await page.route('**/api/v1/pods/capabilities', (r) =>
      r.fulfill({ json: { logs: true, exec: true, delete: false } }),
    )
    await page.goto('/app/pods')
    await page.getByLabel('Namespace').selectOption('zorvia-system')
    const notice = page.getByTestId('pods-rbac-notice')
    await expect(notice).toContainText('Restart and Delete unavailable')
    await expect(notice).toContainText('pods: delete')
    const row = page.locator('tr', { hasText: 'zorvia-api' }).first()
    await expect(row.getByRole('button', { name: /^Delete zorvia-api/ })).toBeDisabled()
    await expect(row.getByRole('button', { name: /^Restart zorvia-api/ })).toBeDisabled()
    await expect(row.getByRole('button', { name: 'Logs' })).toBeEnabled()
  })

  test('event stream and serial console use Terminal.app black', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 1000 })
    await page.goto('/app/events')
    const events = page.getByTestId('event-stream-terminal')
    await expect(events).toContainText('curl -sN /api/events/stream')
    await page.getByLabel('Filter by level').selectOption('warning')
    await expect(events).toContainText('grep -i warning')
    const bg = await page.locator('.zf-terminal-pro .zf-terminal-body').first().evaluate((el) => getComputedStyle(el).backgroundColor)
    expect(bg).toBe('rgb(0, 0, 0)')
    await page.screenshot({ path: 'test-results/event-stream.png' })

    await page.goto('/app/vms/ubuntu-demo/console')
    const serial = page.getByTestId('vm-serial-console')
    await expect(serial.locator('.xterm')).toBeVisible({ timeout: 20_000 })
    await expect(page.locator('.zf-terminal-pro .zf-terminal-title').first()).toContainText('zorvia console ubuntu-demo')
    await expect(serial.locator('.xterm-rows')).toContainText('Connected to ubuntu-demo', { timeout: 20_000 })
    await serial.locator('.xterm').click()
    await page.keyboard.press('Enter')
    await expect(serial.locator('.xterm-rows')).toContainText('login:', { timeout: 15_000 })
    await page.screenshot({ path: 'test-results/vm-console.png' })
  })

  test('light and dark theme screenshots', async ({ page }) => {
    test.setTimeout(120_000)
    await page.setViewportSize({ width: 1440, height: 900 })
    for (const theme of ['light', 'dark'] as const) {
      await page.evaluate((t) => localStorage.setItem('zorvia-theme', t), theme)
      const shots: Array<[string, string]> = [
        ['dashboard', '/app'],
        ['vms', '/app/vms'],
      ]
      for (const [name, path] of shots) {
        await page.goto(path, { waitUntil: 'load' })
        await page.waitForTimeout(1500)
        const applied = await page.evaluate(() => document.documentElement.getAttribute('data-theme'))
        expect(applied === 'dark').toBe(theme === 'dark')
        await page.screenshot({ path: `test-results/theme-${theme}-${name}.png` })
      }
      await page.goto('/app/vms', { waitUntil: 'load' })
      await page.waitForTimeout(1500)
      const vmLink = page.locator('a[href^="/app/vms/"]').first()
      if (await vmLink.count()) {
        await vmLink.click()
        await page.waitForTimeout(2000)
        await page.screenshot({ path: `test-results/theme-${theme}-vm-detail.png` })
      }
    }
  })
})

test.describe('marketing pages', () => {
  test.skip(!livePass, 'Set ZORVIA_E2E_PASSWORD for live lab crawl')

  test('public pages load', async ({ page }) => {
    for (const path of MARKETING_PATHS) {
      // SPA navigations can abort prior loads; retry once on ERR_ABORTED.
      for (let attempt = 0; attempt < 2; attempt++) {
        try {
          await page.goto(path, { waitUntil: 'commit' })
          break
        } catch (e) {
          const msg = e instanceof Error ? e.message : String(e)
          if (!msg.includes('ERR_ABORTED') || attempt === 1) throw e
        }
      }
      await expect(page.locator('body')).toBeVisible()
    }
  })
})
