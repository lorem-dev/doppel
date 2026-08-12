import { expect, test, type Page } from '@playwright/test'

import { PRIVATE_CONFIG, ROOT_TOKEN } from '../src/configs'
import { paintedRgb, paintedRgbAll } from '../src/colour'
import { startDoppel, type Doppel } from '../src/server'

// Syntax colouring, measured in painted pixels.
//
// prism's grammars ship in its package and its palette does not, so for a while
// every editor here tokenised perfectly and rendered in one colour --
// indistinguishable from no highlighting at all. A test for a class name would have
// passed throughout. These read the colour a screen actually gets.

let doppel: Doppel

test.beforeAll(async () => {
  doppel = await startDoppel(PRIVATE_CONFIG)
})
test.afterAll(() => doppel.stop())

/** Signed in as admin. */
async function signIn(page: Page): Promise<void> {
  await page.goto(doppel.baseURL)
  await page.getByRole('textbox', { name: 'Token' }).fill(ROOT_TOKEN)
  await page.getByRole('button', { name: 'Use this token' }).click()
  await page.getByText('root (admin)').waitFor()
}

/** Every distinct painted colour among the highlighter's spans. */
async function tokenColours(page: Page): Promise<Set<string>> {
  const painted = await paintedRgbAll(page, 'pre .token', 'color')
  expect(painted.length).toBeGreaterThan(0)
  return new Set(painted.map((rgb) => rgb.join(',')))
}

test('the highlighter colours JSON and markup, in both themes', async ({ page }) => {
  await signIn(page)
  await page
    .getByRole('row', { name: /alpha/ })
    .getByRole('link', { name: 'Templates' })
    .click()
  await page.getByRole('heading', { name: 'Templates for alpha' }).waitFor()
  const editor = page.getByLabel(/^Contents of/)

  await editor.fill('{"greeting": "hello", "count": 12}')
  const base = (await paintedRgb(page, 'pre', 'color')).join(',')
  const light = await tokenColours(page)

  // More than one colour, and not all of them the text's own: with no palette
  // every token is `base` and both of these fail.
  expect(light.size).toBeGreaterThan(1)
  expect([...light].some((colour) => colour !== base)).toBe(true)

  // The dark palette is a separate theme in the package, re-scoped under `.dark`;
  // without it a dark page would show the light theme's colours.
  await page.getByLabel('Theme').selectOption('dark')
  const dark = await tokenColours(page)
  expect(dark.size).toBeGreaterThan(1)
  expect([...dark].some((colour) => !light.has(colour))).toBe(true)

  // The other grammar. `text` has none by design -- it is text -- so the two that
  // colour are the two asserted.
  await page.getByLabel('Kind').selectOption('html.j2')
  await editor.fill('<p class="greeting">hello</p>')
  expect((await tokenColours(page)).size).toBeGreaterThan(1)
})

test('a path pattern is coloured, and sits at the height of a control', async ({ page }) => {
  // The field an operator gets wrong most often, so its brackets, classes and
  // quantifiers are worth colouring -- a stray bracket is visible before the server
  // compiles it.
  await signIn(page)
  await page.getByRole('button', { name: 'Add a proxy' }).click()
  await page.locator('summary').filter({ hasText: 'Mocks' }).first().click()
  await page.getByRole('button', { name: 'Add a mock' }).click()

  const pattern = page.getByLabel('Path pattern')
  await pattern.fill('^/api/v1/resource/(?P<resourceId>\\d+)/$')

  const colours = await tokenColours(page)
  // Groups, classes, quantifiers and anchors are four different things, and prism's
  // regex grammar aliases each onto a class the theme colours.
  expect(colours.size).toBeGreaterThan(2)

  // One line, at the height of the select beside it: an editor two pixels short of
  // its neighbours is the stepping this form has already been fixed for.
  const [box, method] = await Promise.all([
    pattern.boundingBox(),
    page.getByLabel(/method/).boundingBox(),
  ])
  expect(box!.height).toBe(method!.height)
})

test('the whole document is coloured in the YAML editor', async ({ page }) => {
  await signIn(page)
  await page.getByRole('link', { name: 'alpha' }).click()
  await page.getByRole('heading', { name: 'Edit alpha' }).waitFor()
  await page.getByLabel('Edit as YAML').check()

  const editor = page.getByLabel('The whole proxy, as YAML')
  await editor.fill('name: alpha\ntype: http\ntimeout: 30\nrewrite_redirects: false\n')

  // Keys, strings, a number and a boolean are four things, and prism's yaml grammar
  // aliases each onto a class the theme colours.
  expect((await tokenColours(page)).size).toBeGreaterThan(2)
})

test('a Jinja expression is coloured everywhere one is allowed', async ({ page }) => {
  // Four fields render through Jinja: a mock's text body, its JSON body, a response
  // header, and a template file. The braces used to be plain in all of them, which made
  // `{{ requestId }}` look like part of the text it is not.
  await signIn(page)
  await page.getByRole('button', { name: 'Add a proxy' }).click()
  await page.locator('summary').filter({ hasText: 'Mocks' }).first().click()
  await page.getByRole('button', { name: 'Add a mock' }).click()

  // A header value: one line, and a template rather than a string.
  await page.getByRole('button', { name: 'Add Response headers' }).click()
  await page.getByLabel('Response headers header name 1').fill('X-Request-ID')
  await page.getByLabel('Response headers template 1').fill('rid-{{ requestId }}')
  const header = await tokenColours(page)
  expect(header.size).toBeGreaterThan(1)

  // A text body: the same, with room for a statement.
  await page.getByLabel(/mock-1 body/).fill('hello {{ name }}{% if late %}, sorry{% endif %}')
  expect((await tokenColours(page)).size).toBeGreaterThan(1)

  // A JSON body: both languages at once, which is the composition worth checking in a
  // browser rather than only in a unit test -- an expression inside a string is the
  // case prism resolves in favour of the string unless it is asked not to.
  await page.getByLabel('mock-1 response source').selectOption('json')
  await page.getByLabel(/mock-1 json/).fill('{"id": "{{ resourceId }}", "n": 1}')
  const both = await paintedRgbAll(page, 'pre .token', 'color')
  const json = await paintedRgbAll(page, 'pre .token.property', 'color')
  const jinja = await paintedRgbAll(page, 'pre .token.jinja2 .token.variable', 'color')
  expect(json.length).toBeGreaterThan(0)
  expect(jinja.length).toBeGreaterThan(0)
  expect(json[0]).not.toEqual(jinja[0])
  expect(new Set(both.map((rgb) => rgb.join(','))).size).toBeGreaterThan(2)
})
