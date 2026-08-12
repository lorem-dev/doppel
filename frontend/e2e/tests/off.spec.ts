import { expect, test } from '@playwright/test'

import { DASHBOARD_OFF_CONFIG } from '../src/configs'
import { startDoppel, type Doppel } from '../src/server'

let doppel: Doppel

test.beforeAll(async () => {
  doppel = await startDoppel(DASHBOARD_OFF_CONFIG)
})
test.afterAll(() => doppel.stop())

test('admin.dashboard: false leaves the three routes unrouted', async ({ page }) => {
  for (const path of ['/', '/robots.txt', '/static/anything.js']) {
    const response = await page.request.get(`${doppel.baseURL}${path}`)
    expect(response.status(), path).toBe(404)
    expect((await response.json()).code, path).toBe('NOT_FOUND')
  }
})

test('the JSON API is untouched', async ({ page }) => {
  // The flag is about the dashboard, not about the listener: `admin.enable` is
  // the one that turns the whole thing off.
  const response = await page.request.get(`${doppel.baseURL}/api/v1/proxies`)
  expect(response.status()).toBe(200)
  expect((await response.json()).proxies).toHaveLength(2)
})
