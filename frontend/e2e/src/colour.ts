// Measuring what a screen actually shows.
//
// Two specs need this: one asks whether the page's own colours are readable, the
// other whether the syntax highlighter is colouring anything at all. Both are
// questions about painted pixels rather than about class names, which is the point
// -- the failures they guard were a dark theme rendering white on white and a
// highlighter tokenising in one colour, and a class assertion would have passed
// through both.

import type { Page } from '@playwright/test'

/** sRGB bytes. */
export type Rgb = [number, number, number]

/**
 * What a screen gets for an element's colour, as sRGB bytes.
 *
 * Measured by painting the computed value into a canvas and reading the pixel
 * back, rather than by parsing the string. Tailwind 4 emits `oklch(...)`, Chrome
 * returns it from `getComputedStyle` verbatim, and assigning it to `fillStyle`
 * keeps it in that space -- only a paint converts it. The converted pixel is also
 * the honest subject of a contrast assertion, since it is what a reader sees.
 */
export async function paintedRgb(
  page: Page,
  selector: string,
  property: 'color' | 'backgroundColor',
): Promise<Rgb> {
  return page.locator(selector).first().evaluate(paint, property)
}

/**
 * The same measurement for every element a selector matches.
 *
 * Waits for one match before measuring. `all()` returns whatever matches at that
 * instant and waits for nothing, while the spans this is pointed at -- the
 * highlighter's -- appear on a React update rather than with the keystroke that
 * caused them. Measured too early, a caller's "more than zero colours" assertion
 * fails for a reason that has nothing to do with colour: the same one-shot read
 * that made `fieldMessage` fail on CI and never on a laptop.
 *
 * So this is for asking about elements that are expected to be there. A test that
 * wants to prove something is *not* highlighted should assert on the locator's
 * count instead of measuring the colours of nothing.
 */
export async function paintedRgbAll(
  page: Page,
  selector: string,
  property: 'color' | 'backgroundColor',
): Promise<Rgb[]> {
  await page.locator(selector).first().waitFor({ state: 'attached' })
  return Promise.all(
    (await page.locator(selector).all()).map((node) => node.evaluate(paint, property)),
  )
}

/** Runs in the browser: the computed value, through a canvas, out as bytes. */
function paint(node: Element, which: string): Rgb {
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
  return [r, g, b] as Rgb
}

/** The relative luminance of sRGB bytes, per WCAG. */
export function luminance([r, g, b]: Rgb): number {
  const channel = (value: number) => {
    const v = value / 255
    return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4
  }
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

/** WCAG contrast between two painted colours. */
export function contrast(one: Rgb, two: Rgb): number {
  const [light, dark] = [luminance(one), luminance(two)].sort((a, b) => b - a) as [number, number]
  return (light + 0.05) / (dark + 0.05)
}
