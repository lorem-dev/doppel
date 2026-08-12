import { expect, test } from '@playwright/test'

import { PRIVATE_CONFIG } from '../src/configs'
import { startDoppel, type Doppel } from '../src/server'

let doppel: Doppel

test.beforeAll(async () => {
  doppel = await startDoppel(PRIVATE_CONFIG)
})
test.afterAll(() => doppel.stop())

test('the header shows admin.title and the tab takes it too', async ({ page }) => {
  await page.goto(doppel.baseURL)
  await expect(page.getByRole('heading', { name: 'Doppel (e2e)' })).toBeVisible()
  await expect(page).toHaveTitle('Doppel (e2e)')
})

test('the footer names the copyright and the version of the binary', async ({ page }) => {
  await page.goto(doppel.baseURL)
  // The version comes from the running process, so this also catches a page
  // served by a binary other than the one under test.
  await expect(page.getByText(/\(c\) 2026 Lorem Dev/)).toBeVisible()
  await expect(page.getByText(/Doppel \d+\.\d+\.\d+/)).toBeVisible()
})

test('the theme choice survives a reload', async ({ page }) => {
  await page.goto(doppel.baseURL)
  await page.getByLabel('Theme').selectOption('dark')
  await expect(page.locator('html')).toHaveClass(/dark/)

  await page.reload()
  // Read from localStorage on the way back up: a choice that did not survive a
  // reload would be a choice nobody bothers making twice.
  await expect(page.getByLabel('Theme')).toHaveValue('dark')
  await expect(page.locator('html')).toHaveClass(/dark/)
})

test('the dashboard refuses to be indexed', async ({ page }) => {
  const response = await page.goto(doppel.baseURL)
  expect(response?.headers()['x-robots-tag']).toContain('noindex')

  const robots = await page.request.get(`${doppel.baseURL}/robots.txt`)
  expect(await robots.text()).toContain('Disallow: /')
})

test('navigation is real: a proxy has its own URL and the back button works', async ({ page }) => {
  // The reason react-router is here rather than a state variable.
  await page.goto(doppel.baseURL)
  // The dialog is a fixed overlay, so it has to be out of the way before anything
  // underneath it can be clicked -- which is also what an operator does.
  await page.getByRole('button', { name: 'Continue without a token' }).click()
  await page.getByRole('link', { name: 'alpha' }).click()
  await expect(page).toHaveURL(/\/proxies\/alpha$/)
  await expect(page.getByRole('heading', { name: 'Edit alpha' })).toBeVisible()

  await page.goBack()
  await expect(page.getByRole('heading', { name: 'Proxies' })).toBeVisible()
})
