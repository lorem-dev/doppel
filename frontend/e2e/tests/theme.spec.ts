import { expect, test, type Page } from '@playwright/test'

import { PRIVATE_CONFIG, ROOT_TOKEN } from '../src/configs'
import { startDoppel, type Doppel } from '../src/server'

let doppel: Doppel

test.beforeAll(async () => {
  doppel = await startDoppel(PRIVATE_CONFIG)
})
test.afterAll(() => doppel.stop())

/**
 * What a screen actually gets for an element's colour, as sRGB bytes.
 *
 * Measured by painting the computed value into a canvas and reading the pixel
 * back, rather than by parsing the string. Tailwind 4 emits `oklch(...)`, Chrome
 * returns it from `getComputedStyle` verbatim, and assigning it to `fillStyle`
 * keeps it in that space -- only a paint converts it. The converted pixel is also
 * the honest subject of a contrast assertion, since it is what a reader sees.
 */
async function paintedRgb(
  page: Page,
  selector: string,
  property: 'color' | 'backgroundColor',
): Promise<[number, number, number]> {
  return page.locator(selector).evaluate((node, which) => {
    const computed = getComputedStyle(node)[which as 'color']
    const canvas = document.createElement('canvas')
    canvas.width = 1
    canvas.height = 1
    const context = canvas.getContext('2d')
    if (!context) {
      throw new Error('no 2d context to paint a colour into')
    }
    context.fillStyle = computed
    context.fillRect(0, 0, 1, 1)
    const [r, g, b] = context.getImageData(0, 0, 1, 1).data
    return [r, g, b] as [number, number, number]
  }, property)
}

/** The relative luminance of sRGB bytes, per WCAG. */
function luminance([r, g, b]: [number, number, number]): number {
  const channel = (value: number) => {
    const v = value / 255
    return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4
  }
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

/** WCAG contrast between two painted colours. */
function contrast(one: [number, number, number], two: [number, number, number]): number {
  const [light, dark] = [luminance(one), luminance(two)].sort((a, b) => b - a) as [number, number]
  return (light + 0.05) / (dark + 0.05)
}

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
