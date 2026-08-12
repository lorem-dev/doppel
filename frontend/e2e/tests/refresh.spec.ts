import { expect, test } from '@playwright/test'

import { PRIVATE_CONFIG, ROOT_TOKEN } from '../src/configs'
import { startDoppel, type Doppel } from '../src/server'

let doppel: Doppel

test.beforeAll(async () => {
  doppel = await startDoppel(PRIVATE_CONFIG)
})
test.afterAll(() => doppel.stop())

test('the list refetches once a minute without being asked', async ({ page }) => {
  // The clock is installed before the page loads, so the interval the app sets is
  // the one being fast-forwarded.
  await page.clock.install()
  await page.goto(doppel.baseURL)
  await page.getByRole('textbox', { name: 'Token' }).fill(ROOT_TOKEN)
  await page.getByRole('button', { name: 'Use this token' }).click()
  await expect(page.getByRole('cell', { name: 'alpha', exact: true })).toBeVisible()

  // Changed out from under the page, through the API, exactly as another operator
  // or a `config push` would.
  const created = await page.request.post(`${doppel.baseURL}/api/v1/proxies`, {
    headers: { 'X-Proxy-Authorization': `Bearer ${ROOT_TOKEN}` },
    data: {
      proxy: {
        name: 'gamma',
        type: 'http',
        url: 'https://gamma.example.com/',
        // `alpha` is the default proxy already, and only one may be.
        resolve: { type: 'header', header: 'X-Proxy-Name' },
      },
    },
  })
  expect(created.status()).toBe(201)
  await expect(page.getByRole('cell', { name: 'gamma', exact: true })).toBeHidden()

  await page.clock.runFor('01:05')
  await expect(page.getByRole('cell', { name: 'gamma', exact: true })).toBeVisible()
})
