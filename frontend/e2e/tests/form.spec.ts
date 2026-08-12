import { expect, test, type Page } from '@playwright/test'

import { PRIVATE_CONFIG, ROOT_TOKEN } from '../src/configs'
import { fieldMessage } from '../src/fields'
import { startDoppel, type Doppel } from '../src/server'

let doppel: Doppel

test.beforeAll(async () => {
  doppel = await startDoppel(PRIVATE_CONFIG)
})
test.afterAll(() => doppel.stop())

/**
 * Unfold a section of the form.
 *
 * Every group starts folded, so a scenario has to open what it is about to fill in
 * -- which is also what an operator does, and the reason the test says which
 * sections it touches.
 */
async function open(page: Page, title: string) {
  // The summary itself, not the group: a section that contains another one -- Mocks
  // holds the variable maps -- matches its child's text too.
  await page.locator('summary').filter({ hasText: title }).first().click()
}

async function signedIn(page: Page) {
  await page.goto(doppel.baseURL)
  await page.getByRole('textbox', { name: 'Token' }).fill(ROOT_TOKEN)
  await page.getByRole('button', { name: 'Use this token' }).click()
  await page.getByText('root (admin)').waitFor()
}

/** The stored document, read back through the API rather than off the screen. */
async function stored(page: Page, name: string): Promise<Record<string, unknown>> {
  const response = await page.request.get(
    `${doppel.baseURL}/api/v1/proxies/${encodeURIComponent(name)}`,
    { headers: { 'X-Proxy-Authorization': `Bearer ${ROOT_TOKEN}` } },
  )
  expect(response.status()).toBe(200)
  const { proxy } = (await response.json()) as { proxy: Record<string, unknown> }
  return proxy
}

/**
 * Every field of a proxy, filled in through the form, and the document that comes
 * out of it.
 *
 * The form covers the whole of `ProxyConfig`, and the unit suite only checks that
 * each field is *mentioned*: a control wired to the wrong key, or one whose value
 * never reaches the document, passes that and fails here. This is also the only
 * test that exercises the form the way it is used -- one long sitting, every
 * section, then save.
 *
 * Read back through the API on purpose. Asserting on the screen would prove the
 * form can display what was typed into it, which is not the question.
 */
test('every field of a proxy can be filled in and is stored', async ({ page }) => {
  await signedIn(page)
  await page.getByRole('button', { name: 'Add a proxy' }).click()

  // Identity and upstream.
  await page.getByLabel('Name', { exact: true }).fill('gamma')
  await page.getByLabel('Upstream URL').fill('https://gamma.example.com/api/')
  await expect(page.getByLabel('Type')).toHaveValue('http')
  await expect(page.getByLabel('Type')).toBeDisabled()

  // Forwarding.
  await open(page, 'Forwarding')
  await page.getByLabel('Timeout (seconds)').fill('12')
  await page.getByLabel('Body limit').fill('4Mi')
  // Resolved by header, because `alpha` is already the default proxy and rule V12
  // allows only one.
  await page.getByLabel('Resolve by').selectOption('header')
  await page.getByLabel('Header name').fill('X-Proxy-Name')

  const headers = page.getByRole('button', { name: 'Add Headers sent upstream' })
  await headers.click()
  await page.getByLabel('Headers sent upstream header name 1').fill('X-Injected')
  await page.getByLabel('Headers sent upstream value 1').fill('yes')

  // Faults.
  await open(page, 'Faults')
  await page.getByLabel('Replace').fill('0.25')
  await page.getByLabel('Rewrite redirects').selectOption('false')
  await page.getByLabel('Loss rate').fill('0.05')
  await page.getByLabel('Loss status').fill('503')
  await page.getByLabel('Latency rate').fill('0.5')
  await page.getByLabel('Minimum (seconds)').fill('0.1')
  await page.getByLabel('Maximum (seconds)').fill('1.5')

  // Access overrides. `reader` is a token this configuration knows; a name it did
  // not would be refused by rule V27, which is the next test.
  await open(page, 'Access overrides')
  await page.getByLabel('read', { exact: true }).fill('reader')
  await page.getByLabel('upload', { exact: true }).fill('public')

  // One mock, with all three selector maps, a JSON body and an override.
  await open(page, 'Mocks')
  await page.getByRole('button', { name: 'Add a mock' }).click()
  await page.getByLabel('Mock name').fill('one-widget')
  // The mock's own controls are labelled with its name, so they are addressed by
  // the name just typed.
  await page.getByLabel('one-widget method').selectOption('POST')
  await page.getByLabel('Path pattern').fill('^/widgets$')
  await open(page, 'Variables from the request')
  await page.getByRole('button', { name: 'Add Variables from headers' }).click()
  await page.getByLabel('Variables from headers variable 1').fill('trace')
  await page.getByLabel('Variables from headers header name 1').fill('X-Trace-Id')
  await page.getByRole('button', { name: 'Add Variables from the query' }).click()
  await page.getByLabel('Variables from the query variable 1').fill('filter')
  await page.getByLabel('Variables from the query selector 1').fill('.filter')
  await page.getByRole('button', { name: 'Add Variables from the body' }).click()
  await page.getByLabel('Variables from the body variable 1').fill('items')
  await page.getByLabel('Variables from the body selector 1').fill('.content.items')

  // A spinbutton, not the `Status` tab in the header: the accessible names collide
  // and the roles do not.
  await page.getByRole('spinbutton', { name: 'Status', exact: true }).fill('201')
  await page.getByLabel('one-widget response source').selectOption('json')
  await page.getByLabel('one-widget json').fill('{"ok": true}')
  await page.getByRole('button', { name: 'Add Response headers' }).click()
  await page.getByLabel('Response headers header name 1').fill('X-Mocked')
  await page.getByLabel('Response headers template 1').fill('one-widget')

  await page.getByRole('button', { name: 'Create proxy' }).click()
  await expect(page.getByRole('cell', { name: 'gamma', exact: true })).toBeVisible()

  const proxy = await stored(page, 'gamma')
  expect(proxy).toMatchObject({
    name: 'gamma',
    type: 'http',
    url: 'https://gamma.example.com/api/',
    timeout: 12,
    body_limit: 4 * 1024 * 1024,
    replace: 0.25,
    rewrite_redirects: false,
    resolve: { type: 'header', header: 'X-Proxy-Name' },
    headers: { 'X-Injected': 'yes' },
    loss: { percentage: 0.05, status: 503 },
    latency: { percentage: 0.5, min: 0.1, max: 1.5 },
    access: { read: ['reader'], upload: 'public' },
    mocks: [
      {
        name: 'one-widget',
        request: {
          method: 'POST',
          url: '^/widgets$',
          headers: { trace: 'X-Trace-Id' },
          query: { filter: '.filter' },
          body: { items: '.content.items' },
        },
        response: {
          status: 201,
          json: '{"ok": true}',
          headers: { 'X-Mocked': 'one-widget' },
        },
      },
    ],
  })

  // Nothing the form did not touch was invented: `template` and `body` are the two
  // other response sources, and exactly one may be set.
  const [mock] = proxy.mocks as Array<{ response: Record<string, unknown> }>
  expect(mock!.response.body).toBeUndefined()
  expect(mock!.response.template).toBeUndefined()
})

test('what the form loads is what it saves back unchanged', async ({ page }) => {
  // The other half of the form's job. A control that renders a value but writes a
  // different one -- or drops a field it does not know -- shows up as a document
  // that changed without anybody editing it.
  await signedIn(page)
  const before = await stored(page, 'alpha')

  await page.getByRole('link', { name: 'alpha' }).click()
  await page.getByRole('heading', { name: 'Edit alpha' }).waitFor()
  await page.getByRole('button', { name: 'Save changes' }).click()
  await expect(page.getByRole('heading', { name: 'Proxies' })).toBeVisible()

  expect(await stored(page, 'alpha')).toEqual(before)
})

test('a name the configuration does not know is refused, on its field', async ({ page }) => {
  await signedIn(page)
  await page.getByRole('link', { name: 'alpha' }).click()
  await page.getByRole('heading', { name: 'Edit alpha' }).waitFor()

  // Rule V27: `access` may only name a token or group that exists.
  await open(page, 'Access overrides')
  await page.getByLabel('read', { exact: true }).fill('nobody')
  await page.getByRole('button', { name: 'Save changes' }).click()

  await expect(await fieldMessage(page, 'read')).toContainText('nobody')
})

test('a save that removes something says what, and takes no for an answer', async ({ page }) => {
  // A form full of Remove buttons has no undo, and one button commits all of them. The
  // confirmation is on the save rather than on each Remove: taking one mock out and
  // putting another in is a single edit, and asking on the way would be asking about a
  // state still being assembled.
  const doppel = await startDoppel(PRIVATE_CONFIG)
  try {
    await page.goto(doppel.baseURL)
    await page.getByRole('textbox', { name: 'Token' }).fill(ROOT_TOKEN)
    await page.getByRole('button', { name: 'Use this token' }).click()
    await page.getByText('root (admin)').waitFor()

    // Give beta something to lose: a mock, an injected header, an access override.
    await page.getByRole('link', { name: 'beta' }).click()
    await page.locator('summary').filter({ hasText: 'Forwarding' }).first().click()
    await page.getByRole('button', { name: 'Add Headers sent upstream' }).click()
    await page.getByLabel('Headers sent upstream header name 1').fill('X-Injected')
    await page.getByLabel('Headers sent upstream value 1').fill('yes')
    await page.locator('summary').filter({ hasText: 'Mocks' }).first().click()
    await page.getByRole('button', { name: 'Add a mock' }).click()
    await page.getByRole('button', { name: 'Save changes' }).click()
    await page.getByRole('cell', { name: 'beta', exact: true }).waitFor()

    // Now take both away.
    await page.getByRole('link', { name: 'beta' }).click()
    await page.locator('summary').filter({ hasText: 'Forwarding' }).first().click()
    await page.getByRole('button', { name: 'Remove' }).first().click()
    await page.locator('summary').filter({ hasText: 'Mocks' }).first().click()
    await page.getByRole('button', { name: 'Remove mock' }).click()

    // The save asks, and names them.
    await page.getByRole('button', { name: 'Save changes' }).click()
    const dialog = page.getByRole('dialog')
    await expect(dialog).toContainText('the mock `mock-1`')
    await expect(dialog).toContainText('the injected header `X-Injected`')

    // No, and nothing was sent: the fields are still as they were left.
    await dialog.getByRole('button', { name: 'Cancel' }).click()
    await expect(page.getByRole('heading', { name: 'Edit beta' })).toBeVisible()
    const before = await page.request.get(`${doppel.baseURL}/api/v1/proxies/beta`, {
      headers: { 'X-Proxy-Authorization': `Bearer ${ROOT_TOKEN}` },
    })
    expect(JSON.stringify(await before.json())).toContain('mock-1')

    // Yes, and both are gone.
    await page.getByRole('button', { name: 'Save changes' }).click()
    await page.getByRole('dialog').getByRole('button', { name: 'Save changes' }).click()
    await page.getByRole('cell', { name: 'beta', exact: true }).waitFor()

    const after = await page.request.get(`${doppel.baseURL}/api/v1/proxies/beta`, {
      headers: { 'X-Proxy-Authorization': `Bearer ${ROOT_TOKEN}` },
    })
    const stored = JSON.stringify(await after.json())
    expect(stored).not.toContain('mock-1')
    expect(stored).not.toContain('X-Injected')
  } finally {
    doppel.stop()
  }
})

test('a save that only changes things does not ask', async ({ page }) => {
  const doppel = await startDoppel(PRIVATE_CONFIG)
  try {
    await page.goto(doppel.baseURL)
    await page.getByRole('textbox', { name: 'Token' }).fill(ROOT_TOKEN)
    await page.getByRole('button', { name: 'Use this token' }).click()
    await page.getByText('root (admin)').waitFor()
    await page.getByRole('link', { name: 'alpha' }).click()
    await page.getByLabel('Upstream URL').fill('https://moved.example.com/api/')
    await page.getByRole('button', { name: 'Save changes' }).click()

    // Straight through: nothing was removed, so there is nothing to agree to.
    await expect(page.getByRole('cell', { name: 'alpha', exact: true })).toBeVisible()
    await expect(page.getByRole('dialog')).toHaveCount(0)
  } finally {
    doppel.stop()
  }
})
