import { expect, test } from '@playwright/test'

import { PRIVATE_CONFIG, PUBLIC_CONFIG } from '../src/configs'
import { paintedRgb } from '../src/colour'
import { startDoppel, type Doppel } from '../src/server'

let doppel: Doppel

test.beforeAll(async () => {
  doppel = await startDoppel(PRIVATE_CONFIG)
})
test.afterAll(() => doppel.stop())

test("the header shows admin.title, in the wordmark's own shape", async ({ page }) => {
  await page.goto(doppel.baseURL)
  await expect(page.getByRole('heading', { name: 'Doppel (e2e)' })).toBeVisible()
  await expect(page).toHaveTitle('Doppel (e2e)')

  // A named deployment gets its own name -- that is what tells four open tabs
  // apart -- drawn the way `Doppelganger` is, because it is the same heading in the
  // same page and should not read as a different one.
  const heading = page.getByRole('heading', { level: 1 })
  const style = await heading
    .locator('span')
    .first()
    .evaluate((node) => {
      const computed = getComputedStyle(node)
      return { size: computed.fontSize, style: computed.fontStyle, select: computed.userSelect }
    })
  expect(style.size).toBe('18px')
  expect(style.style).toBe('italic')
  expect(style.select).toBe('none')

  // Two tones, split on the space: `Doppel` in the strong one, `(e2e)` in the
  // light one, exactly as the wordmark splits its own two halves.
  const [first, second] = await Promise.all([
    paintedRgb(page, 'h1 span span:nth-child(1)', 'color'),
    paintedRgb(page, 'h1 span span:nth-child(2)', 'color'),
  ])
  expect(first).not.toEqual(second)
  // Every character survives the split.
  await expect(heading).toHaveText('Doppel (e2e)')
})

test('a named deployment says it was built with Doppel, and links to Doppel', async ({ page }) => {
  // The footer belongs to somebody else's tool now: the version line says what it
  // was built with, the documentation link says whose documentation it is, and the
  // repository link goes -- on `billing-api (staging)`, "Repository" reads as a
  // link to the billing API's own, which it is not.
  await page.goto(doppel.baseURL)

  await expect(page.getByText(/Built with Doppel \d+\.\d+\.\d+/)).toBeVisible()
  await expect(page.getByText(/\(c\) \d{4} Lorem Dev/)).toBeVisible()
  await expect(page.getByRole('link', { name: /Doppel Documentation/ })).toBeVisible()
  await expect(page.getByRole('link', { name: /^Repository/ })).toHaveCount(0)
})

test("an unnamed deployment keeps the project's own footer", async ({ page }) => {
  const doppel = await startDoppel(PUBLIC_CONFIG)
  try {
    await page.goto(doppel.baseURL)

    // Nobody named this one, so it is Doppel's own page: the copyright reads as
    // Doppel's and both links are its own.
    await expect(page.getByText(/Built with Doppel/)).toHaveCount(0)
    await expect(page.getByText(/\(c\) \d{4} Lorem Dev . Doppel \d+\.\d+\.\d+/)).toBeVisible()
    await expect(page.getByRole('link', { name: /^Repository/ })).toBeVisible()
    await expect(page.getByRole('link', { name: /Doppel Documentation/ })).toHaveCount(0)
    await expect(page.getByRole('link', { name: /^Documentation/ })).toBeVisible()
  } finally {
    doppel.stop()
  }
})

test('an unnamed deployment gets the wordmark, in two tones and one size', async ({ page }) => {
  // `admin.title` absent. The page then says what this is rather than repeating a
  // default string, and the two halves are the two halves of the name.
  const doppel = await startDoppel(PUBLIC_CONFIG)
  try {
    await page.goto(doppel.baseURL)
    const heading = page.getByRole('heading', { level: 1 })
    await expect(heading).toHaveText('Doppelganger')

    const [first, second] = await Promise.all([
      paintedRgb(page, 'h1 span span:nth-child(1)', 'color'),
      paintedRgb(page, 'h1 span span:nth-child(2)', 'color'),
    ])
    expect(first).not.toEqual(second)

    // The size the heading always was, and italic serif rather than the page's
    // sans: a wordmark that grew would push the tabs along with it.
    const style = await heading.locator('span').first().evaluate((node) => {
      const computed = getComputedStyle(node)
      return {
        size: computed.fontSize,
        style: computed.fontStyle,
        family: computed.fontFamily,
        select: computed.userSelect,
      }
    })
    expect(style.size).toBe('18px')
    expect(style.style).toBe('italic')
    expect(style.family).toMatch(/serif/i)
    // And it takes no selection: a mark that highlights in two pieces when the tab
    // beside it is double-clicked looks like a mistake.
    expect(style.select).toBe('none')

    // The tab keeps the plain default: a browser tab is a list of words.
    await expect(page).toHaveTitle('Doppel')
  } finally {
    doppel.stop()
  }
})

test('the footer puts the copyright and the links on one line, at either end', async ({ page }) => {
  await page.goto(doppel.baseURL)

  const copyright = await page.getByText(/Lorem Dev/).boundingBox()
  const links = await page.locator('footer nav').boundingBox()
  expect(copyright).not.toBeNull()
  expect(links).not.toBeNull()

  // One line: their vertical centres agree.
  const centre = (box: { y: number; height: number }) => box.y + box.height / 2
  expect(Math.abs(centre(copyright!) - centre(links!))).toBeLessThan(2)
  // Either end, in that order.
  expect(copyright!.x).toBeLessThan(links!.x)
})

test('the footer names the copyright and the version of the binary', async ({ page }) => {
  await page.goto(doppel.baseURL)

  // Both come from the running process, so this also catches a page served by a
  // binary other than the one under test. The year is the build's -- read back out
  // of the page it was injected into rather than written here, because a literal
  // would be a second copy of it that goes stale in January.
  const injected = await page.locator('#doppel-config').textContent()
  const { copyrightYear } = JSON.parse(injected ?? '{}') as { copyrightYear: number }
  expect(copyrightYear).toBeGreaterThanOrEqual(2026)

  await expect(page.getByText(`(c) ${copyrightYear} Lorem Dev`)).toBeVisible()
  await expect(page.getByText(/Doppel \d+\.\d+\.\d+/)).toBeVisible()
})

test('the theme choice survives a reload', async ({ page }) => {
  await page.goto(doppel.baseURL)
  await page.getByLabel('Theme').selectOption('dark')
  await expect(page.locator('html')).toHaveClass(/dark/)

  await page.reload()
  // Read from localStorage on the way back up: a choice that did not survive a
  // reload would be a choice nobody bothers making twice.
  await expect(page.getByLabel('Theme')).toHaveValue('dark')
  await expect(page.locator('html')).toHaveClass(/dark/)
})

test('the dashboard refuses to be indexed', async ({ page }) => {
  const response = await page.goto(doppel.baseURL)
  expect(response?.headers()['x-robots-tag']).toContain('noindex')

  const robots = await page.request.get(`${doppel.baseURL}/robots.txt`)
  expect(await robots.text()).toContain('Disallow: /')
})

test('navigation is real: a proxy has its own URL and the back button works', async ({ page }) => {
  // The reason react-router is here rather than a state variable.
  await page.goto(doppel.baseURL)
  // The dialog is a fixed overlay, so it has to be out of the way before anything
  // underneath it can be clicked -- which is also what an operator does.
  await page.getByRole('button', { name: 'Continue without a token' }).click()
  await page.getByRole('link', { name: 'alpha' }).click()
  await expect(page).toHaveURL(/\/proxies\/alpha$/)
  await expect(page.getByRole('heading', { name: 'Edit alpha' })).toBeVisible()

  await page.goBack()
  await expect(page.getByRole('heading', { name: 'Proxies' })).toBeVisible()
})

test('the title goes home, from wherever the form has taken you', async ({ page }) => {
  await page.goto(doppel.baseURL)
  await page.getByRole('button', { name: 'Continue without a token' }).click()
  await page.getByRole('link', { name: 'alpha' }).click()
  await expect(page).toHaveURL(/\/proxies\/alpha$/)

  // The affordance people try first when a form has taken them somewhere they did
  // not mean to be. `Doppel (e2e)` is this deployment's `admin.title`.
  await page.getByRole('link', { name: 'Doppel (e2e)' }).click()
  await expect(page).toHaveURL(new RegExp(`^${doppel.baseURL}/$`))
  await expect(page.getByRole('heading', { name: 'Proxies' })).toBeVisible()
})
