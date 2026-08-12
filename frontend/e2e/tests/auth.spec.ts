import { expect, test } from '@playwright/test'

import { PRIVATE_CONFIG, PUBLIC_CONFIG, ROOT_TOKEN } from '../src/configs'
import { startDoppel, type Doppel } from '../src/server'

test.describe('a private configuration', () => {
  let doppel: Doppel
  test.beforeAll(async () => {
    doppel = await startDoppel(PRIVATE_CONFIG)
  })
  test.afterAll(() => doppel.stop())

  test('asks for a token, and takes no for an answer', async ({ page }) => {
    await page.goto(doppel.baseURL)
    const dialog = page.getByRole('form', { name: 'Admin token' })
    await expect(dialog).toBeVisible()

    await page.getByRole('button', { name: 'Continue without a token' }).click()
    await expect(dialog).toBeHidden()

    // `list` and `read` are public here, so declining leaves a working page
    // rather than an empty one.
    await expect(page.getByRole('cell', { name: 'alpha', exact: true })).toBeVisible()
    // ...and the writes are visibly unavailable rather than absent.
    await expect(page.getByRole('button', { name: 'Add a proxy' })).toBeDisabled()
  })

  test('does not ask again after being declined once', async ({ page }) => {
    await page.goto(doppel.baseURL)
    await page.getByRole('button', { name: 'Continue without a token' }).click()
    await page.reload()
    await expect(page.getByRole('form', { name: 'Admin token' })).toBeHidden()
  })

  test('enables the writes once a token is given, and names the caller', async ({ page }) => {
    await page.goto(doppel.baseURL)
    await page.getByRole('textbox', { name: 'Token' }).fill(ROOT_TOKEN)
    await page.getByRole('button', { name: 'Use this token' }).click()

    await expect(page.getByText('root (admin)')).toBeVisible()
    await expect(page.getByRole('button', { name: 'Add a proxy' })).toBeEnabled()
  })

  test('a wrong token leaves the writes disabled', async ({ page }) => {
    await page.goto(doppel.baseURL)
    await page.getByRole('textbox', { name: 'Token' }).fill('wrong-token-000000000000000000000000')
    await page.getByRole('button', { name: 'Use this token' }).click()

    // An unrecognised token is anonymous, deliberately: telling it apart from a
    // recognised one would confirm which tokens exist.
    await expect(page.getByRole('button', { name: 'Add a proxy' })).toBeDisabled()
  })

  test('signing out returns to the anonymous view without blocking the page', async ({ page }) => {
    await page.goto(doppel.baseURL)
    await page.getByRole('textbox', { name: 'Token' }).fill(ROOT_TOKEN)
    await page.getByRole('button', { name: 'Use this token' }).click()
    await expect(page.getByRole('button', { name: 'Add a proxy' })).toBeEnabled()

    await page.getByRole('button', { name: 'Sign out' }).click()
    await expect(page.getByRole('button', { name: 'Add a proxy' })).toBeDisabled()
    // Not a wall: what is public is still on screen.
    await expect(page.getByRole('cell', { name: 'alpha', exact: true })).toBeVisible()
    await expect(page.getByRole('form', { name: 'Admin token' })).toBeHidden()
  })
})

test.describe('a public configuration', () => {
  let doppel: Doppel
  test.beforeAll(async () => {
    doppel = await startDoppel(PUBLIC_CONFIG)
  })
  test.afterAll(() => doppel.stop())

  test('never mentions tokens', async ({ page }) => {
    await page.goto(doppel.baseURL)
    await expect(page.getByRole('form', { name: 'Admin token' })).toBeHidden()
    await expect(page.getByRole('button', { name: 'Sign in' })).toBeHidden()
    // And everything is permitted, because it is.
    await expect(page.getByRole('button', { name: 'Add a proxy' })).toBeEnabled()
  })
})
