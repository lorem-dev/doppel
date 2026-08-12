import { expect, test, type Page } from '@playwright/test'

import { PRIVATE_CONFIG, ROOT_TOKEN } from '../src/configs'
import { startDoppel, type Doppel } from '../src/server'

// What the page refuses before the server is asked.
//
// Every bound checked here is read from `GET /api/v1/schema` at load, which is
// generated from the Rust newtypes that enforce it -- so these are not a second set
// of rules but the same ones, arriving early. The last test in this file is the one
// that proves it: with the schema endpoint unreachable, the complaints are gone and
// saving is the only way to find out, exactly as it was before.

let doppel: Doppel

test.beforeAll(async () => {
  doppel = await startDoppel(PRIVATE_CONFIG)
})
test.afterAll(() => doppel.stop())

/** Signed in as admin, on the form for a new proxy. */
async function openForm(page: Page): Promise<void> {
  await page.goto(doppel.baseURL)
  await page.getByRole('textbox', { name: 'Token' }).fill(ROOT_TOKEN)
  await page.getByRole('button', { name: 'Use this token' }).click()
  await page.getByText('root (admin)').waitFor()
  await page.getByRole('button', { name: 'Add a proxy' }).click()
  await page.getByRole('heading', { name: 'Add a proxy' }).waitFor()
}

/** Open a folded section by its heading. */
async function open(page: Page, title: string): Promise<void> {
  await page.locator('summary').filter({ hasText: title }).first().click()
}

test('a name that breaks the pattern is refused while it is being typed', async ({ page }) => {
  await openForm(page)
  const name = page.getByLabel('Name', { exact: true })

  await name.fill('has space')
  // Nothing was saved: this is the schema's `pattern`, and the message is the
  // type's own description of it.
  await expect(page.getByText('Letters, digits, - and _, between 2 and 32 characters.')).toBeVisible()

  await name.fill('billing-api')
  await expect(
    page.getByText('Letters, digits, - and _, between 2 and 32 characters.'),
  ).toBeHidden()
})

test('a name outside its length is refused, at either end', async ({ page }) => {
  await openForm(page)
  const name = page.getByLabel('Name', { exact: true })

  await name.fill('a')
  await expect(page.getByText('at least 2 characters')).toBeVisible()

  await name.fill('x'.repeat(33))
  await expect(page.getByText('at most 32 characters; this is 33')).toBeVisible()

  await name.fill('x'.repeat(32))
  await expect(page.getByText(/at most 32 characters/)).toBeHidden()
})

test('a number outside its range is refused, and the input carries the range', async ({ page }) => {
  await openForm(page)
  await open(page, 'Forwarding')

  const timeout = page.getByLabel('Timeout (seconds)')
  // The bounds the browser itself enforces on the spinners and the keyboard, taken
  // from the schema rather than written here.
  await expect(timeout).toHaveAttribute('min', '1')
  await expect(timeout).toHaveAttribute('max', '3600')

  await timeout.fill('5000')
  await expect(page.getByText('must be between 1 and 3600')).toBeVisible()

  await timeout.fill('30')
  await expect(page.getByText('must be between 1 and 3600')).toBeHidden()
})

test('a fraction that was written as a percentage is refused', async ({ page }) => {
  // The mistake the schema's own documentation names: `percentage: 45` where a
  // fraction was meant.
  await openForm(page)
  await open(page, 'Faults')

  await page.getByLabel('Loss rate').fill('45')
  await expect(page.getByText('must be between 0 and 1')).toBeVisible()

  await page.getByLabel('Loss rate').fill('0.05')
  await expect(page.getByText('must be between 0 and 1')).toBeHidden()
})

test('a mock is held to the same bounds as the proxy around it', async ({ page }) => {
  await openForm(page)
  await open(page, 'Mocks')
  await page.getByRole('button', { name: 'Add a mock' }).click()

  // A mock's name allows 64 characters where a proxy's allows 32, and the page
  // knows the difference because it read both.
  await page.getByLabel('Mock name').fill('two words')
  await expect(page.getByText('Letters, digits, - and _, between 2 and 64 characters.')).toBeVisible()
  await page.getByLabel('Mock name').fill('one-widget')
  await expect(
    page.getByText('Letters, digits, - and _, between 2 and 64 characters.'),
  ).toBeHidden()

  // Exact: "Loss status" contains "Status" too, and carries the same bounds.
  const status = page.getByLabel('Status', { exact: true })
  await expect(status).toHaveAttribute('min', '100')
  await expect(status).toHaveAttribute('max', '599')
  await status.fill('99')
  await expect(page.getByText('must be between 100 and 599')).toBeVisible()
})

test('a selector is checked as it is typed', async ({ page }) => {
  await openForm(page)
  await open(page, 'Mocks')
  await page.getByRole('button', { name: 'Add a mock' }).click()
  await open(page, 'Variables from the request')

  await page.getByRole('button', { name: 'Add Variables from the body' }).click()
  const selector = page.getByLabel('Variables from the body selector 1')

  // The leading dot is the whole rule, and forgetting it is the mistake.
  await selector.fill('content.items')
  await expect(page.getByText(/leading dot/)).toBeVisible()
  await expect(selector).toHaveAttribute('aria-invalid', 'true')

  await selector.fill('.content.items')
  await expect(page.getByText(/leading dot/)).toBeHidden()
})

test('a path pattern is left to the server, dialect and all', async ({ page }) => {
  // Deliberately unchecked here. The server compiles it with Rust's `regex`, whose
  // named groups are `(?P<id>...)` -- which `new RegExp` refuses. A browser-side
  // syntax check would report perfectly good Doppel patterns as broken, so the only
  // honest answer is the server's.
  await openForm(page)
  await open(page, 'Mocks')
  await page.getByRole('button', { name: 'Add a mock' }).click()

  await page.getByLabel('Path pattern').fill('/api/v1/resource/(?P<resourceId>\\d+)/')
  // No complaint anywhere on the form: the field's own hint still says what a path
  // pattern is, and nothing has been reported as wrong.
  await expect(page.getByRole('alert')).toHaveCount(0)
})

test('the bounds come from the server, not from this bundle', async ({ page }) => {
  // The test that keeps the rest of this file honest. With the schema unreachable
  // the page has nothing to check against, so a name it would otherwise refuse
  // draws no complaint -- which is only possible if the complaint came from the
  // fetched document.
  await page.route('**/api/v1/schema', (route) => route.abort())
  await openForm(page)

  await page.getByLabel('Name', { exact: true }).fill('has space')
  await expect(
    page.getByText('Letters, digits, - and _, between 2 and 32 characters.'),
  ).toHaveCount(0)

  // And the form still works: the server is what refuses it.
  await page.getByLabel('Upstream URL').fill('https://example.com/api/')
  await page.getByRole('button', { name: 'Create proxy' }).click()
  await expect(page.getByRole('alert').first()).toContainText(/name/i)
})

test('an (i) beside a field links to that field, for this version', async ({ page }) => {
  // A wrong fragment is silent in a browser -- the page just opens at the top -- so
  // this asserts the whole href, and `scripts/check_docs_links.py` asserts that the
  // fragment exists in the generated reference.
  await openForm(page)

  const injected = await page.locator('#doppel-config').textContent()
  const { version } = JSON.parse(injected ?? '{}') as { version: string }

  // Located by where it goes: every (i) is named the same on purpose, so that
  // looking a field up by its label finds the field rather than the link beside it.
  const url = (anchor: string) =>
    `https://lorem-dev.github.io/doppel/${version}/usage/parameters/#${anchor}`

  const name = page.locator(`a[href="${url('proxies-name')}"]`)
  await expect(name).toBeVisible()
  await expect(name).toHaveAttribute('title', /^Name:/)

  // A footnote marker beside the words, not a control: small, and centred on the line
  // rather than on the space under it. It sat two pixels low when the row centred it
  // against the label's bottom margin.
  const mark = (await name.boundingBox())!
  const label = (await page.getByText('Name', { exact: true }).boundingBox())!
  expect(mark.height).toBeLessThanOrEqual(14)
  expect(Math.abs(mark.y + mark.height / 2 - (label.y + label.height / 2))).toBeLessThan(1)
  // It leaves the dashboard, and a half-filled form is not worth losing.
  await expect(name).toHaveAttribute('target', '_blank')

  await open(page, 'Faults')
  await expect(page.locator(`a[href="${url('proxies-loss-percentage')}"]`)).toBeVisible()
})
