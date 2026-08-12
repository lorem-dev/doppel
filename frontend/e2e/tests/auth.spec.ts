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

test.describe('a refusal that only a token can fix', () => {
  // Nothing is public here, so every read is refused until a token is presented --
  // which is the state the Refresh button exists for.
  const CLOSED = `
server:
  host: "127.0.0.1"
  port: {proxyPort}
admin:
  host: "127.0.0.1"
  port: {adminPort}
  tokens:
    - name: root
      group: admin
      token: ${ROOT_TOKEN}
  access:
    list: ["admin"]
    read: ["admin"]
    create: ["admin"]
    update: ["admin"]
    delete: ["admin"]
    upload: ["admin"]
  upload:
    limit: 1Mi
control:
  socket: {controlSocket}
templates:
  dir: {templatesDir}
proxies:
  - name: alpha
    type: http
    url: "https://alpha.example.com/api/"
`

  let doppel: Doppel
  test.beforeAll(async () => {
    doppel = await startDoppel(CLOSED)
  })
  test.afterAll(() => doppel.stop())

  test('the proxy form offers a token and a refresh, and both work', async ({ page }) => {
    // A form reads once, when it opens. Signing in afterwards used to leave the
    // refusal on screen with nothing to press: the fix was reloading the browser.
    await page.goto(`${doppel.baseURL}/proxies/alpha`)
    await page.getByRole('button', { name: 'Continue without a token' }).click()

    const refusal = page.getByRole('alert').first()
    await expect(refusal).toContainText(/requires access|token/i)
    await expect(refusal.getByRole('button', { name: 'Refresh' })).toBeVisible()

    // The other half of the offer: the thing that makes a refresh worth pressing.
    await refusal.getByRole('button', { name: 'Enter token' }).click()
    await page.getByRole('textbox', { name: 'Token' }).fill(ROOT_TOKEN)
    await page.getByRole('button', { name: 'Use this token' }).click()

    // Still nothing read, so the form is still the empty one it renders behind the
    // refusal...
    await expect(page.getByLabel('Upstream URL')).toHaveValue('')
    // ...and now it has been read.
    await page.getByRole('button', { name: 'Refresh' }).click()
    await expect(page.getByLabel('Upstream URL')).toHaveValue('https://alpha.example.com/api/')
    await expect(page.getByRole('alert')).toHaveCount(0)
  })

  test('the list offers the same, and a public deployment offers no token', async ({ page }) => {
    await page.goto(doppel.baseURL)
    await page.getByRole('button', { name: 'Continue without a token' }).click()

    const refusal = page.getByRole('alert').first()
    await expect(refusal.getByRole('button', { name: 'Refresh' })).toBeVisible()
    await expect(refusal.getByRole('button', { name: 'Enter token' })).toBeVisible()
  })
})
