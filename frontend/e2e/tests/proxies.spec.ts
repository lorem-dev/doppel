import { expect, test } from '@playwright/test'

import { PRIVATE_CONFIG, READER_TOKEN, ROOT_TOKEN } from '../src/configs'
import { startDoppel, type Doppel } from '../src/server'

let doppel: Doppel

test.beforeAll(async () => {
  doppel = await startDoppel(PRIVATE_CONFIG)
})
test.afterAll(() => doppel.stop())

/** Arrive holding a token, without going through the dialog every time. */
async function signedIn(
  page: import('@playwright/test').Page,
  token = ROOT_TOKEN,
  caller = 'root (admin)',
) {
  await page.goto(doppel.baseURL)
  await page.getByRole('textbox', { name: 'Token' }).fill(token)
  await page.getByRole('button', { name: 'Use this token' }).click()
  // Waiting for the caller's name rather than for the table: the table is there
  // anonymously too, so it proves nothing about the token having been accepted,
  // and every later assertion about a control's state depends on the rights
  // report having arrived.
  await expect(page.getByText(caller)).toBeVisible()
}

test('the list shows what each proxy does', async ({ page }) => {
  await signedIn(page)
  const row = page.getByRole('row', { name: /alpha/ })
  await expect(row).toContainText('https://alpha.example.com/api/')
  await expect(row).toContainText('default')
})

test('a proxy can be created, edited and deleted', async ({ page }) => {
  await signedIn(page)

  await page.getByRole('button', { name: 'Add a proxy' }).click()
  await page.getByLabel('Name').fill('gamma')
  await page.getByLabel('Upstream URL').fill('https://gamma.example.com/')
  // Resolved by header, because `alpha` is already the default proxy and rule V12
  // allows only one. Leaving this alone is how an operator meets that rule for
  // the first time -- as a refusal from the server, which is the next test.
  //
  // Inside a folded section, so it is opened first: every group on the form starts
  // folded, which is what keeps the form to one screen.
  await page.locator('summary').filter({ hasText: 'Forwarding' }).first().click()
  await page.getByLabel('Resolve by').selectOption('header')
  await page.getByLabel('Header name').fill('X-Proxy-Name')
  await page.getByRole('button', { name: 'Create proxy' }).click()
  await expect(page.getByRole('cell', { name: 'gamma', exact: true })).toBeVisible()

  await page.getByRole('link', { name: 'gamma' }).click()
  await page.getByLabel('Upstream URL').fill('https://moved.example.com/')
  await page.getByRole('button', { name: 'Save changes' }).click()
  await expect(page.getByRole('row', { name: /gamma/ })).toContainText('moved.example.com')

  await page.getByRole('row', { name: /gamma/ }).getByRole('button', { name: 'Delete' }).click()
  await page.getByRole('dialog').getByRole('button', { name: 'Delete' }).click()
  await expect(page.getByRole('cell', { name: 'gamma', exact: true })).toBeHidden()
})

test("a rejected document lands on the field the server complained about", async ({ page }) => {
  await signedIn(page)
  await page.getByRole('button', { name: 'Add a proxy' }).click()
  await page.getByLabel('Name').fill('bad')
  // Not a URL Doppel accepts as an upstream. The server is the arbiter of that,
  // and its complaint has to arrive under the field it is about rather than only
  // in the banner.
  await page.getByLabel('Upstream URL').fill('not-a-url')
  await page.getByRole('button', { name: 'Create proxy' }).click()

  // Under the field, not merely somewhere on the page. Asserting on the banner
  // alone is what let a parser that never matched anything pass: the banner
  // carries the same words.
  const field = page.getByText('Upstream URL').locator('..')
  await expect(field.getByRole('alert')).toContainText('must be absolute')
  // And nothing was created.
  await page.goto(doppel.baseURL)
  await expect(page.getByRole('cell', { name: 'bad', exact: true })).toBeHidden()
})

test('a read-only token sees the writes disabled with the reason', async ({ page }) => {
  await signedIn(page, READER_TOKEN, 'reader (user)')

  const create = page.getByRole('button', { name: 'Add a proxy' })
  await expect(create).toBeDisabled()
  // Disabled rather than hidden, and it says why: a missing button reads as a
  // broken page.
  await expect(create).toHaveAttribute('title', /may not create/)

  const remove = page.getByRole('row', { name: /alpha/ }).getByRole('button', { name: 'Delete' })
  await expect(remove).toBeDisabled()
})

test('a template is written from the page and listed afterwards', async ({ page }) => {
  await signedIn(page)

  // A template file has to be declared by a mock before it may be uploaded --
  // the server answers 422 TEMPLATE_NOT_DECLARED otherwise. Declared through the
  // API rather than the form, so this spec stays about templates.
  const read = await page.request.get(`${doppel.baseURL}/api/v1/proxies/alpha`, {
    headers: { 'X-Proxy-Authorization': `Bearer ${ROOT_TOKEN}` },
  })
  const { revision, proxy } = (await read.json()) as {
    revision: string
    proxy: Record<string, unknown>
  }
  const declared = await page.request.put(`${doppel.baseURL}/api/v1/proxies/alpha`, {
    headers: {
      'X-Proxy-Authorization': `Bearer ${ROOT_TOKEN}`,
      'If-Match': `"${revision}"`,
    },
    data: {
      revision,
      proxy: {
        ...proxy,
        mocks: [
          {
            name: 'rendered',
            request: { method: 'GET', url: '^/rendered$' },
            response: { status: 200, template: 'page.json.j2' },
          },
        ],
      },
    },
  })
  expect(declared.status()).toBe(200)

  await page.reload()
  await page.getByRole('row', { name: /alpha/ }).getByRole('link', { name: 'Templates' }).click()

  await page.getByLabel('File name').fill('page')
  await page.getByLabel('Kind').selectOption('json.j2')
  // The editor is a lazy chunk, so it arrives after the page does.
  const editor = page.getByLabel('Contents of page.json.j2')
  await editor.fill('{"ok": true}')
  await page.getByRole('button', { name: /Save page\.json\.j2/ }).click()

  // The file's own row: the editor's label and the save button both mention the
  // name, so the assertion has to be about the table.
  await expect(page.getByRole('cell', { name: 'page.json.j2', exact: true })).toBeVisible()
})
