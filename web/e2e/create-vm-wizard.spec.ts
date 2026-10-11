// Copyright 2026 Zyvor AI Labs · Apache-2.0
import { test, expect } from '@playwright/test'

// Regression: the wizard's "Next" and "Create" buttons share one DOM slot. Without
// distinct React keys the browser submitted the form on the Resources -> Review "Next",
// creating the VM before the Review step was shown. The API is mocked, so this runs
// against the preview build with no cluster.
test('Next into Review does not create the VM; Create submits exactly once', async ({ page }) => {
  let vmPosts = 0
  await page.addInitScript(() => localStorage.setItem('zorvia_token', 'test'))
  await page.route('**/api/**', async (route) => {
    const req = route.request()
    const path = new URL(req.url()).pathname
    if (req.method() === 'POST' && path === '/api/vms') {
      vmPosts++
      return route.fulfill({ status: 201, contentType: 'application/json', body: JSON.stringify({ name: 'wizard-vm', state: 'starting' }) })
    }
    if (path.includes('auth/me')) {
      return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ id: '1', role: 'admin', username: 'admin' }) })
    }
    const isList = /\/(vms|images|templates|nodes|networks|devices|zones|snapshots|backups|catalog|pods|alerts)\/?$/.test(path)
    return route.fulfill({ status: 200, contentType: 'application/json', body: isList ? '[]' : '{}' })
  })

  await page.goto('/app/create')
  await page.locator('#vm-name').fill('wizard-vm')
  await page.locator('#vm-image').fill('quay.io/kubevirt/cirros-container-disk-demo')
  await page.getByRole('button', { name: /^next$/i }).click()
  await page.locator('#vm-memory').fill('1024')
  await page.locator('#vm-disk').fill('2')
  await page.getByRole('button', { name: /^next$/i }).click()

  const create = page.getByRole('button', { name: /create virtual machine/i })
  await expect(create).toBeVisible()
  expect(vmPosts, 'Next must not submit the form').toBe(0)

  await create.click()
  await expect.poll(() => vmPosts).toBe(1)
})
