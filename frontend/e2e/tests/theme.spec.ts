import { expect, test } from '@playwright/test'

import { PRIVATE_CONFIG, ROOT_TOKEN } from '../src/configs'
import { contrast, luminance, paintedRgb } from '../src/colour'
import { startDoppel, type Doppel } from '../src/server'

let doppel: Doppel

test.beforeAll(async () => {
  doppel = await startDoppel(PRIVATE_CONFIG)
})
test.afterAll(() => doppel.stop())

/**
 * The page's own colours, in both themes.
 *
 * This exists because the dark theme rendered a white page: every component
 * carried its own `dark:` text colour and nothing coloured the page itself, so the
 * heading was near-white on white. A screenshot showed it immediately and no
 * assertion did -- which is why this one is about contrast rather than about a
 * class being present.
 */
for (const theme of ['light', 'dark'] as const) {
  test(`the ${theme} theme is readable`, async ({ page }) => {
    await page.goto(doppel.baseURL)
    await page.getByRole('button', { name: 'Continue without a token' }).click()
    await page.getByLabel('Theme').selectOption(theme)

    const background = await paintedRgb(page, 'body', 'backgroundColor')
    const heading = await paintedRgb(page, 'h1', 'color')

    // 4.5:1 is the WCAG threshold for body text. The failure this guards measured
    // about 1.1:1.
    expect(contrast(background, heading)).toBeGreaterThan(4.5)

    // And the page is the colour the theme says, rather than merely contrasting:
    // white text on a white page fails the check above, but so would a light theme
    // that had gone dark.
    const brightness = luminance(background)
    if (theme === 'light') {
      expect(brightness).toBeGreaterThan(0.5)
    } else {
      expect(brightness).toBeLessThan(0.1)
    }

    // Native controls -- a select, a scrollbar, a caret -- are drawn by the browser
    // and follow this and nothing else.
    const scheme = await page.locator('html').evaluate((node) => getComputedStyle(node).colorScheme)
    expect(scheme).toBe(theme)
  })
}

test('a notice goes away on its own', async ({ page }) => {
  // They were dismissible and nothing else, so three reloads left three notices
  // stacked in the corner for the rest of the session.
  await page.clock.install()
  await page.goto(doppel.baseURL)
  await page.getByRole('textbox', { name: 'Token' }).fill(ROOT_TOKEN)
  await page.getByRole('button', { name: 'Use this token' }).click()
  await page.getByText('root (admin)').waitFor()

  await page.getByRole('link', { name: 'Status' }).click()
  await page.getByRole('button', { name: 'Reload configuration' }).click()

  const notice = page.getByRole('button', { name: /Reloaded at revision/ })
  await expect(notice).toBeVisible()
  await page.clock.runFor('00:05')
  await expect(notice).toBeHidden()
})

test('the form opens folded, and its save bar is on screen from the start', async ({ page }) => {
  // Two complaints with one cause: the form was a page and a half of controls, so
  // the save button was a scroll away and the section being edited was buried under
  // the ones that were not.
  await page.setViewportSize({ width: 1200, height: 600 })
  await page.goto(doppel.baseURL)
  await page.getByRole('textbox', { name: 'Token' }).fill(ROOT_TOKEN)
  await page.getByRole('button', { name: 'Use this token' }).click()
  await page.getByText('root (admin)').waitFor()
  await page.getByRole('link', { name: 'alpha' }).click()
  await page.getByRole('heading', { name: 'Edit alpha' }).waitFor()

  // Folded: the fields inside a section are not on the page until it is opened.
  await expect(page.getByLabel('Replace')).toBeHidden()
  await expect(page.getByLabel('Timeout (seconds)')).toBeHidden()
  // And a folded section still says whether there is anything in it.
  await expect(page.locator('summary').filter({ hasText: 'Faults' })).toContainText('none')

  const save = page.getByRole('button', { name: 'Save changes' })
  const box = await save.boundingBox()
  const viewport = page.viewportSize()
  expect(box).not.toBeNull()
  // On screen without scrolling, which is what "sticky" has to mean.
  expect(box!.y + box!.height).toBeLessThanOrEqual(viewport!.height)

  // Still on screen with everything open, which is the case that used to fail.
  for (const section of ['Forwarding', 'Faults', 'Access overrides', 'Mocks']) {
    await page.locator('summary').filter({ hasText: section }).first().click()
  }
  const opened = await save.boundingBox()
  expect(opened!.y + opened!.height).toBeLessThanOrEqual(viewport!.height)
})

test('the controls on a row are the same height, and a select has an arrow', async ({ page }) => {
  // The elements "jumped": an input, a select and a button sat at three heights, so
  // a row of them stepped up and down.
  await page.goto(doppel.baseURL)
  await page.getByRole('textbox', { name: 'Token' }).fill(ROOT_TOKEN)
  await page.getByRole('button', { name: 'Use this token' }).click()
  await page.getByText('root (admin)').waitFor()
  await page.getByRole('link', { name: 'alpha' }).click()
  await page.locator('summary').filter({ hasText: 'Forwarding' }).first().click()

  const heights = await Promise.all(
    [
      page.getByLabel('Name', { exact: true }),
      page.getByLabel('Timeout (seconds)'),
      page.getByLabel('Resolve by'),
      page.getByRole('button', { name: 'Save changes' }),
    ].map(async (locator) => (await locator.boundingBox())!.height),
  )
  expect(new Set(heights).size).toBe(1)

  // The select's own affordance. `appearance-none` removed the native arrow, and a
  // background image replaced it -- for a while it did not, because the arbitrary
  // Tailwind value carrying the SVG never reached the stylesheet, leaving a select
  // with no arrow at all.
  const select = page.getByLabel('Resolve by')
  await expect(select).toHaveCSS('appearance', 'none')
  const image = await select.evaluate((node) => getComputedStyle(node).backgroundImage)
  expect(image).toContain('url(')
})

test('signing in clears a refusal that is already on screen', async ({ page }) => {
  // The list is fetched on a timer, so a refusal shown to an anonymous caller used
  // to stay up until the next tick -- up to a minute of the page telling someone who
  // had just entered a token that they needed one.
  const doppel = await startDoppel(`
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
`)
  try {
    await page.goto(doppel.baseURL)
    // Nothing is public here, so declining leaves a page that says so.
    await page.getByRole('button', { name: 'Continue without a token' }).click()
    const refusal = page.getByRole('alert').first()
    await expect(refusal).toBeVisible()

    await page.getByRole('button', { name: 'Enter token' }).click()
    await page.getByRole('textbox', { name: 'Token' }).fill(ROOT_TOKEN)
    await page.getByRole('button', { name: 'Use this token' }).click()

    // Without waiting for a tick: the refusal goes and the list arrives.
    await expect(page.getByRole('cell', { name: 'alpha', exact: true })).toBeVisible({
      timeout: 5000,
    })
    await expect(refusal).toBeHidden()
  } finally {
    doppel.stop()
  }
})

test('the header links to Swagger UI, and it answers', async ({ page }) => {
  await page.goto(doppel.baseURL)
  await page.getByRole('button', { name: 'Continue without a token' }).click()

  const link = page.getByRole('link', { name: /API/ })
  await expect(link).toHaveAttribute('href', '/api/swagger-ui/')
  // A real link rather than a client-side route, so the target has to answer: it is
  // served by the server, under the prefix react-router must not claim.
  const served = await page.request.get(`${doppel.baseURL}/api/swagger-ui/`)
  expect(served.status()).toBe(200)
  expect(await served.text()).toContain('swagger')
})
