import { expect, test, type Page } from '@playwright/test'

import { PRIVATE_CONFIG, ROOT_TOKEN } from '../src/configs'
import { startDoppel, type Doppel } from '../src/server'

// The console, which an operator reads to find their own problems.
//
// Ours were in the way: `react-simple-code-editor` renders a `<style>` element per
// editor, the page's policy allowed no inline stylesheet, and every editor logged a
// violation -- dozens of them in a session, with nothing visibly wrong. The policy now
// names that one stylesheet by hash, and this is what keeps the console empty.

let doppel: Doppel

test.beforeAll(async () => {
  doppel = await startDoppel(PRIVATE_CONFIG)
})
test.afterAll(() => doppel.stop())

/** Everything the page complains about, and every policy violation it causes. */
function watch(page: Page): { messages: string[] } {
  const messages: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'error' || message.type() === 'warning') {
      messages.push(`${message.type()}: ${message.text()}`)
    }
  })
  page.on('pageerror', (error) => messages.push(`uncaught: ${error.message}`))
  return { messages }
}

test('editing a proxy logs nothing to the console', async ({ page }) => {
  const seen = watch(page)

  await page.goto(doppel.baseURL)
  await page.getByRole('textbox', { name: 'Token' }).fill(ROOT_TOKEN)
  await page.getByRole('button', { name: 'Use this token' }).click()
  await page.getByText('root (admin)').waitFor()
  await page.getByRole('link', { name: 'alpha' }).click()
  await page.getByRole('heading', { name: 'Edit alpha' }).waitFor()

  // Every section, because each editor used to cost a violation of its own.
  for (const section of ['Forwarding', 'Faults', 'Access overrides', 'Mocks']) {
    await page.locator('summary').filter({ hasText: section }).first().click()
  }
  await page.getByRole('button', { name: 'Add a mock' }).click()
  await page.getByLabel('mock-1 response source').selectOption('json')
  await page.getByLabel(/mock-1 json/).fill('{"id": "{{ id }}"}')
  await page.getByLabel('Edit as YAML').check()
  await page.getByLabel('The whole proxy, as YAML').waitFor()

  expect(seen.messages).toEqual([])
})

test('the policy still forbids an inline stylesheet it does not know', async ({ page }) => {
  // The hash allows one stylesheet, not the practice. A page that let anything style
  // itself would pass the test above and lose what the policy is for.
  const seen = watch(page)
  await page.goto(doppel.baseURL)
  await page.getByRole('button', { name: 'Continue without a token' }).click()

  await page.evaluate(() => {
    const style = document.createElement('style')
    style.textContent = 'body { display: none }'
    document.head.append(style)
  })

  expect(seen.messages.join('\n')).toContain('Content Security Policy')
  // And it was refused rather than applied.
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible()
})
