import { expect, test, type Page } from '@playwright/test'

import { PRIVATE_CONFIG, ROOT_TOKEN } from '../src/configs'
import { startDoppel, type Doppel } from '../src/server'

// Template files are the admin API's, not the dashboard's.
//
// They had a section in the form for a while: a list, an editor, and a name to type.
// It was the wrong shape -- a file store inside a form over a document, with a Save of
// its own that did not wait for the form's -- so it is gone. What the page still does
// is read a mock that answers with a template: it shows which file, refuses to edit
// it, and lets the mock be moved onto a body instead.

let doppel: Doppel

test.beforeAll(async () => {
  doppel = await startDoppel(PRIVATE_CONFIG)
})
test.afterAll(() => doppel.stop())

async function signedIn(page: Page): Promise<void> {
  await page.goto(doppel.baseURL)
  await page.getByRole('textbox', { name: 'Token' }).fill(ROOT_TOKEN)
  await page.getByRole('button', { name: 'Use this token' }).click()
  await page.getByText('root (admin)').waitFor()
}

/** Give `beta` a mock that answers with a template, through the API. */
async function withTemplateMock(page: Page): Promise<void> {
  const read = await page.request.get(`${doppel.baseURL}/api/v1/proxies/beta`, {
    headers: { 'X-Proxy-Authorization': `Bearer ${ROOT_TOKEN}` },
  })
  const { revision, proxy } = (await read.json()) as {
    revision: string
    proxy: Record<string, unknown>
  }
  const written = await page.request.put(`${doppel.baseURL}/api/v1/proxies/beta`, {
    headers: { 'X-Proxy-Authorization': `Bearer ${ROOT_TOKEN}`, 'If-Match': `"${revision}"` },
    data: {
      revision,
      proxy: {
        ...proxy,
        mocks: [
          {
            name: 'from-a-file',
            request: { method: 'GET', url: '^/from-a-file$' },
            response: { status: 200, template: 'page.json.j2' },
          },
        ],
      },
    },
  })
  expect(written.status()).toBe(200)
}

test('the form has no templates section and no way to name a file', async ({ page }) => {
  await signedIn(page)
  await page.getByRole('link', { name: 'alpha' }).click()
  await page.getByRole('heading', { name: 'Edit alpha' }).waitFor()

  await expect(page.locator('summary').filter({ hasText: 'Templates' })).toHaveCount(0)

  // And a new mock is offered two answers, not three.
  await page.locator('summary').filter({ hasText: 'Mocks' }).first().click()
  await page.getByRole('button', { name: 'Add a mock' }).click()
  await expect(page.getByLabel('mock-1 response source').locator('option')).toHaveText([
    'Text body',
    'JSON body',
  ])
})

test('a template set in the configuration is shown, locked, and can be left', async ({ page }) => {
  await withTemplateMock(page)
  await signedIn(page)
  await page.getByRole('link', { name: 'beta' }).click()
  await page.getByRole('heading', { name: 'Edit beta' }).waitFor()
  await page.locator('summary').filter({ hasText: 'Mocks' }).first().click()

  // Shown, so an operator knows what the mock answers with...
  const file = page.getByLabel('Template file')
  await expect(file).toHaveValue('page.json.j2')
  // ...and not editable, because naming a file the page cannot upload would be an
  // invitation to a render error.
  await expect(file).toBeDisabled()
  await expect(page.getByText(/uploaded through the admin API/)).toBeVisible()

  // The mock can be moved onto a body, which is the change the page does support.
  await page.getByLabel('from-a-file response source').selectOption('json')
  await expect(page.getByLabel('Template file')).toHaveCount(0)
  await page.getByLabel(/from-a-file json/).fill('{"ok": true}')

  // Leaving the template is a removal, so the save says so before doing it.
  await page.getByRole('button', { name: 'Save changes' }).click()
  const dialog = page.getByRole('dialog')
  await expect(dialog).toContainText('page.json.j2')
  await dialog.getByRole('button', { name: 'Save changes' }).click()

  const stored = await page.request.get(`${doppel.baseURL}/api/v1/proxies/beta`, {
    headers: { 'X-Proxy-Authorization': `Bearer ${ROOT_TOKEN}` },
  })
  const { proxy } = (await stored.json()) as {
    proxy: { mocks: Array<{ response: Record<string, unknown> }> }
  }
  expect(proxy.mocks[0]!.response.template).toBeUndefined()
  expect(proxy.mocks[0]!.response.json).toBe('{"ok": true}')
})

test('switching away from a template can be undone while the page is open', async ({ page }) => {
  // The name cannot be typed back in, so the option stays offered for as long as the
  // page remembers what the document said.
  await withTemplateMock(page)
  await signedIn(page)
  await page.getByRole('link', { name: 'beta' }).click()
  await page.locator('summary').filter({ hasText: 'Mocks' }).first().click()

  const source = page.getByLabel('from-a-file response source')
  await source.selectOption('body')
  await expect(page.getByLabel('Template file')).toHaveCount(0)

  await source.selectOption('template')
  await expect(page.getByLabel('Template file')).toHaveValue('page.json.j2')
})

test('the old templates address still lands somewhere', async ({ page }) => {
  // It was a page of its own for two releases, so the URL is linkable and someone has
  // it.
  await page.goto(`${doppel.baseURL}/proxies/alpha/templates`)
  await expect(page).toHaveURL(/\/proxies\/alpha$/)
})
