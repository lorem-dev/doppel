import { expect, test, type Page } from '@playwright/test'

import { PRIVATE_CONFIG, ROOT_TOKEN } from '../src/configs'
import { startDoppel, type Doppel } from '../src/server'

let doppel: Doppel

test.beforeAll(async () => {
  doppel = await startDoppel(PRIVATE_CONFIG)
})
test.afterAll(() => doppel.stop())

/** Signed in as admin, on alpha's templates page. Writing needs `upload`. */
async function openTemplates(page: Page): Promise<void> {
  await page.goto(doppel.baseURL)
  await page.getByRole('textbox', { name: 'Token' }).fill(ROOT_TOKEN)
  await page.getByRole('button', { name: 'Use this token' }).click()
  await page.getByText('root (admin)').waitFor()
  await page
    .getByRole('row', { name: /alpha/ })
    .getByRole('link', { name: 'Templates' })
    .click()
  await page.getByRole('heading', { name: 'Templates for alpha' }).waitFor()
}

test('the file name and the kind sit on one line', async ({ page }) => {
  // They did not. The row was a flex row aligned on its bottom edge, and a field
  // with a hint under it is taller than one without -- so the two controls ended up
  // at different heights, with the select hanging below the input it belongs beside.
  // A grid aligns the tops, which is the edge the eye reads a row from.
  await openTemplates(page)

  const name = await page.getByLabel('File name').boundingBox()
  const kind = await page.getByLabel('Kind').boundingBox()

  expect(name).not.toBeNull()
  expect(kind).not.toBeNull()
  expect(kind!.height).toBe(name!.height)
  // Same top edge, to the pixel. The failure this guards was about twenty off.
  expect(Math.abs(kind!.y - name!.y)).toBeLessThan(1)
})

test('the template editor opens at the size of a document', async ({ page }) => {
  // It opened one line tall, sized to its empty content, which reads as a text
  // input for something nobody writes on one line.
  await openTemplates(page)

  const editor = page.getByLabel(/^Contents of/)
  const box = await editor.boundingBox()
  expect(box!.height).toBeGreaterThan(250)

  // And it takes what a textarea takes, newlines included.
  await editor.fill('line one\nline two\nline three')
  await expect(editor).toHaveValue('line one\nline two\nline three')
})
