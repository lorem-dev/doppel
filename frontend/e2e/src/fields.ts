// Asking a field about itself, without walking the DOM.
//
// A form's message lives near its control, and "near" used to mean a specific number
// of parent elements -- which broke the moment the label moved to make room for the
// (i). The control names its message with `aria-describedby`, which is both how a
// screen reader finds it and a stable handle for a test.

import { expect, type Locator, type Page } from '@playwright/test'

/**
 * The hint-or-complaint line under the field with this label.
 *
 * `[id="..."]` rather than `#...`: React's generated ids contain characters a CSS
 * id selector cannot carry.
 */
export async function fieldMessage(page: Page, label: string): Promise<Locator> {
  const described = await page.getByLabel(label).first().getAttribute('aria-describedby')
  expect(described, `the field labelled "${label}" describes no message`).toBeTruthy()
  return page.locator(`[id="${described}"]`)
}
