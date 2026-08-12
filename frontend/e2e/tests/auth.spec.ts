import { expect, test } from '@playwright/test'

import { PRIVATE_CONFIG, PUBLIC_CONFIG, READER_TOKEN, ROOT_TOKEN } from '../src/configs'
import { startDoppel, type Doppel } from '../src/server'

test.describe('a private configuration', () => {
  let doppel: Doppel
  test.beforeAll(async () => {
    doppel = await startDoppel(PRIVATE_CONFIG)
  })
  test.afterAll(() => doppel.stop())

  test('asks for a token, and takes no for an answer', async ({ page }) => {
    await page.goto(doppel.baseURL)
    const dialog = page.getByRole('form', { name: 'Access token' })
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
    await expect(page.getByRole('form', { name: 'Access token' })).toBeHidden()
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

  test('the API reads the token from the header, and says whose it is', async ({ page }) => {
    // Tokens through the API rather than through the page: this is what a script,
    // a CI job or `curl` does, and it is the half the dashboard tests cannot see.
    const asRoot = await page.request.get(`${doppel.baseURL}/api/v1/access`, {
      headers: { 'X-Proxy-Authorization': `Bearer ${ROOT_TOKEN}` },
    })
    expect(asRoot.status()).toBe(200)
    const root = (await asRoot.json()) as {
      caller: { kind: string; name?: string; group?: string }
      global: Record<string, boolean>
    }
    expect(root.caller).toMatchObject({ kind: 'token', name: 'root', group: 'admin' })
    expect(root.global.create).toBe(true)

    // A read-only token: recognised, named, and refused the writes rather than the
    // reads.
    const asReader = await page.request.get(`${doppel.baseURL}/api/v1/access`, {
      headers: { 'X-Proxy-Authorization': `Bearer ${READER_TOKEN}` },
    })
    const reader = (await asReader.json()) as {
      caller: { kind: string; name?: string }
      global: Record<string, boolean>
    }
    expect(reader.caller).toMatchObject({ kind: 'token', name: 'reader' })
    expect(reader.global.read).toBe(true)
    expect(reader.global.create).toBe(false)
  })

  test('an unrecognised token is anonymous, not an error', async ({ page }) => {
    // Deliberately indistinguishable from sending nothing: answering differently
    // would confirm which tokens exist.
    const wrong = await page.request.get(`${doppel.baseURL}/api/v1/access`, {
      headers: { 'X-Proxy-Authorization': `Bearer wrong-token-000000000000000000000000` },
    })
    const none = await page.request.get(`${doppel.baseURL}/api/v1/access`)

    expect(wrong.status()).toBe(200)
    expect(await wrong.json()).toEqual(await none.json())
    expect(((await wrong.json()) as { caller: { kind: string } }).caller.kind).toBe('anonymous')
  })

  test('a write with a token that may not write is refused, with the action named', async ({
    page,
  }) => {
    for (const [token, expected] of [
      [READER_TOKEN, 403],
      [undefined, 401],
    ] as const) {
      const response = await page.request.post(`${doppel.baseURL}/api/v1/proxies`, {
        headers: token ? { 'X-Proxy-Authorization': `Bearer ${token}` } : {},
        data: {
          proxy: {
            name: 'refused',
            type: 'http',
            url: 'https://example.com/',
            resolve: { type: 'header', header: 'X-Proxy-Name' },
          },
        },
      })

      expect(response.status(), `${token ?? 'no token'}`).toBe(expected)
      const body = (await response.json()) as { code: string; message: string }
      // 401 for "who are you", 403 for "not you": the pair a client acts on
      // differently, and the message names the action rather than the resource.
      expect(body.code).toBe(expected === 401 ? 'UNAUTHORIZED' : 'FORBIDDEN')
      expect(body.message).toContain('create')
    }

    // And nothing was written.
    const list = await page.request.get(`${doppel.baseURL}/api/v1/proxies`, {
      headers: { 'X-Proxy-Authorization': `Bearer ${ROOT_TOKEN}` },
    })
    expect(await list.text()).not.toContain('refused')
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
    await expect(page.getByRole('form', { name: 'Access token' })).toBeHidden()
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
    await expect(page.getByRole('form', { name: 'Access token' })).toBeHidden()
    await expect(page.getByRole('button', { name: 'Sign in' })).toBeHidden()
    // And everything is permitted, because it is.
    await expect(page.getByRole('button', { name: 'Add a proxy' })).toBeEnabled()
  })

  test('a token left in this browser changes nothing, and is not offered again', async ({
    page,
  }) => {
    // There is no way to set a token from the page here -- no dialog, no Sign in --
    // so the only route left is a token this browser kept from a deployment that
    // was private earlier, or one somebody put there by hand. It must not resurrect
    // the sign-in flow: `public: true` means the API is unauthenticated, and a page
    // asking for a token it cannot use is a page asking for a secret for nothing.
    await page.goto(doppel.baseURL)
    await page.evaluate(() =>
      window.localStorage.setItem(
        'doppel.token',
        // The shape `services/auth.ts` writes: the value and when it was entered.
        JSON.stringify({ token: 'left-over-token-00000000000000000000', savedAt: Date.now() }),
      ),
    )
    await page.reload()

    await expect(page.getByRole('form', { name: 'Access token' })).toBeHidden()
    await expect(page.getByRole('button', { name: 'Sign in' })).toBeHidden()
    await expect(page.getByRole('button', { name: 'Sign out' })).toBeHidden()
    // Still anonymous, still allowed everything: the caller the API reports is the
    // anonymous one, not a name from a token nobody checked.
    await expect(page.getByText('(admin)')).toBeHidden()
    await expect(page.getByRole('button', { name: 'Add a proxy' })).toBeEnabled()
  })

  test('the API ignores a token instead of refusing it', async ({ page }) => {
    // Both directions, because "unauthenticated" has to mean the header is not
    // consulted rather than that a wrong value is punished: a client that keeps
    // sending its old token must not start getting 401s when a deployment goes
    // public.
    const withToken = await page.request.get(`${doppel.baseURL}/api/v1/access`, {
      headers: { 'X-Proxy-Authorization': 'Bearer not-a-token-0000000000000000000' },
    })
    expect(withToken.status()).toBe(200)
    const report = (await withToken.json()) as {
      caller: { kind: string }
      global: Record<string, boolean>
    }
    expect(report.caller.kind).toBe('anonymous')
    expect(Object.values(report.global).every(Boolean)).toBe(true)

    // And a write goes through with no token at all.
    const created = await page.request.post(`${doppel.baseURL}/api/v1/proxies`, {
      data: {
        proxy: {
          name: 'no-token',
          type: 'http',
          url: 'https://example.com/',
          // By header, because `alpha` is already the default resolver and rule
          // V12 allows only one.
          resolve: { type: 'header', header: 'X-Proxy-Name' },
        },
      },
    })
    expect(created.status(), await created.text()).toBe(201)
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
