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
  const control = page.getByLabel(label).first()
  // Waited for rather than read once. A message that arrives with the server's
  // refusal is not on the page the instant the button is clicked, so a one-shot
  // read is a race a slower machine loses: CI failed here on a run where the whole
  // suite took twice as long as it does on a laptop, holding `null` while the
  // complaint was still in flight. The assertion that follows this one always
  // retried, which is why this was the one that fell over.
  await expect(control, `the field labelled "${label}" describes no message`).toHaveAttribute(
    'aria-describedby',
    /\S/,
  )
  const described = await control.getAttribute('aria-describedby')
  return page.locator(`[id="${described}"]`)
}
