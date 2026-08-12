import { expect, test, type Page } from '@playwright/test'

import { PRIVATE_CONFIG, ROOT_TOKEN } from '../src/configs'
import { startDoppel, type Doppel } from '../src/server'

// Editing a proxy as YAML.
//
// The form is better for changing one field and worse for everything else -- pasting
// a proxy from a colleague, reordering mocks, copying a block out of `main.yaml`.
// This is the same document in the shape the configuration file has.

let doppel: Doppel

test.beforeAll(async () => {
  doppel = await startDoppel(PRIVATE_CONFIG)
})
test.afterAll(() => doppel.stop())

/** Signed in as admin, editing alpha, in YAML mode. */
async function openYaml(page: Page): Promise<void> {
  await page.goto(doppel.baseURL)
  await page.getByRole('textbox', { name: 'Token' }).fill(ROOT_TOKEN)
  await page.getByRole('button', { name: 'Use this token' }).click()
  await page.getByText('root (admin)').waitFor()
  await page.getByRole('link', { name: 'alpha' }).click()
  await page.getByRole('heading', { name: 'Edit alpha' }).waitFor()
  await page.getByLabel('Edit as YAML').check()
  await page.getByLabel('The whole proxy, as YAML').waitFor()
}

const editor = (page: Page) => page.getByLabel('The whole proxy, as YAML')

test('the toggle swaps the form for the document it was editing', async ({ page }) => {
  await openYaml(page)

  // The document, not a template: what the form had loaded.
  await expect(editor(page)).toHaveValue(/name: alpha/)
  await expect(editor(page)).toHaveValue(/url: https:\/\/alpha\.example\.com\/api\//)
  // And the form is gone rather than hidden behind it.
  await expect(page.getByLabel('Upstream URL')).toHaveCount(0)

  // Back again, with the edit carried over.
  await editor(page).fill('name: alpha\ntype: http\nurl: https://beta.example.com/api/\n')
  await page.getByLabel('Edit as YAML').uncheck()
  await expect(page.getByLabel('Upstream URL')).toHaveValue('https://beta.example.com/api/')
})

test('tab indents inside the document instead of leaving it', async ({ page }) => {
  await openYaml(page)
  const yaml = editor(page)

  await yaml.fill('name: alpha\ntype: http\nurl: https://alpha.example.com/api/\nheaders:\n')
  await yaml.press('End')
  await yaml.press('Tab')
  await yaml.pressSequentially('X-A: b')

  // The tab went into the document, so the header is nested under `headers`.
  await expect(yaml).toHaveValue(/headers:\n\s+X-A: b/)
  // And focus never left the editor, which is what a tab usually does.
  await expect(yaml).toBeFocused()
})

test('Reformat tidies what is there', async ({ page }) => {
  await openYaml(page)
  const yaml = editor(page)

  // Legal YAML, written the way nobody wants to read it.
  await yaml.fill('{name: alpha, type: http, url: "https://alpha.example.com/api/"}')
  await page.getByRole('button', { name: 'Reformat' }).click()

  await expect(yaml).toHaveValue('name: alpha\ntype: http\nurl: https://alpha.example.com/api/\n')
})

test('a document that breaks a rule is refused before the server sees it', async ({ page }) => {
  await openYaml(page)
  const yaml = editor(page)
  const save = page.getByRole('button', { name: 'Save changes' })

  await yaml.fill(
    'name: alpha\ntype: http\nurl: https://alpha.example.com/api/\ntimeout: 5000\n',
  )
  // The bound is the schema's, and the message names the field and the number.
  await expect(page.getByRole('alert')).toContainText('timeout')
  await expect(page.getByRole('alert')).toContainText('5000 is greater than 3600')
  await expect(save).toBeDisabled()

  // A field the schema does not know, which is the mistake a typo makes.
  await yaml.fill('name: alpha\ntype: http\nurl: https://alpha.example.com/api/\ntimeuot: 30\n')
  await expect(page.getByRole('alert')).toContainText('is not a field of this document')
  await expect(save).toBeDisabled()

  // Not YAML at all.
  await yaml.fill('name: alpha\n  bad indent: yes\n')
  await expect(page.getByRole('alert')).toBeVisible()
  await expect(save).toBeDisabled()

  // And a good document clears all of it.
  await yaml.fill('name: alpha\ntype: http\nurl: https://alpha.example.com/api/\ntimeout: 45\n')
  await expect(page.getByRole('alert')).toHaveCount(0)
  await expect(save).toBeEnabled()
})

test('the toggle stays put while the document is broken', async ({ page }) => {
  // Switching back would show the form the last document that parsed, quietly
  // throwing away whatever is being written.
  await openYaml(page)
  await editor(page).fill('name: alpha\n  bad indent: yes\n')
  await expect(page.getByLabel('Edit as YAML')).toBeDisabled()
})

test('what the editor saves is what the server stores', async ({ page }) => {
  await openYaml(page)
  await editor(page).fill(
    'name: alpha\ntype: http\nurl: https://alpha.example.com/api/\ntimeout: 45\nreplace: 0.5\n',
  )
  await page.getByRole('button', { name: 'Save changes' }).click()
  await page.getByRole('cell', { name: 'alpha', exact: true }).waitFor()

  const stored = await page.request.get(`${doppel.baseURL}/api/v1/proxies/alpha`, {
    headers: { 'X-Proxy-Authorization': `Bearer ${ROOT_TOKEN}` },
  })
  const { proxy } = (await stored.json()) as { proxy: Record<string, unknown> }
  expect(proxy.timeout).toBe(45)
  expect(proxy.replace).toBe(0.5)
})

test('a refused save leaves the document tidied rather than as it was typed', async ({ page }) => {
  // "Format on save" is only visible when the save comes back refused, which is also
  // when it matters: what is on screen is what has to be fixed.
  await openYaml(page)

  // Move the revision under the form, so the save is refused for a reason that has
  // nothing to do with the document.
  const current = await page.request.get(`${doppel.baseURL}/api/v1/proxies/alpha`, {
    headers: { 'X-Proxy-Authorization': `Bearer ${ROOT_TOKEN}` },
  })
  const { revision, proxy } = (await current.json()) as {
    revision: string
    proxy: Record<string, unknown>
  }
  const moved = await page.request.put(`${doppel.baseURL}/api/v1/proxies/alpha`, {
    headers: {
      'X-Proxy-Authorization': `Bearer ${ROOT_TOKEN}`,
      'If-Match': `"${revision}"`,
      'Content-Type': 'application/json',
    },
    data: { proxy: { ...proxy, timeout: 11 } },
  })
  expect(moved.status()).toBe(200)

  await editor(page).fill('{name: alpha, type: http, url: "https://alpha.example.com/api/"}')
  await page.getByRole('button', { name: 'Save changes' }).click()

  await expect(page.getByRole('alert').first()).toContainText(/revision|reload/i)
  await expect(editor(page)).toHaveValue(
    'name: alpha\ntype: http\nurl: https://alpha.example.com/api/\n',
  )
})

test('the editor links to the documentation for the running version', async ({ page }) => {
  await openYaml(page)
  const injected = await page.locator('#doppel-config').textContent()
  const { version } = JSON.parse(injected ?? '{}') as { version: string }

  // By where it goes: the footer has a Documentation link too, and it goes to the
  // root of the same versioned site.
  const link = page.locator(
    `a[href="https://lorem-dev.github.io/doppel/${version}/usage/parameters/#proxies"]`,
  )
  await expect(link).toBeVisible()
  await expect(link).toHaveAttribute('target', '_blank')
})
