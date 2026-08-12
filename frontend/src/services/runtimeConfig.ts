// What the page was told by the HTML it arrived in.
//
// Read once, at module load. The values come from the admin listener, which
// substitutes them into `index.html` per request, so they describe the process
// serving this page rather than whatever a later fetch might find.

import type { RuntimeConfig } from '../types/api'

const ELEMENT_ID = 'doppel-config'

/**
 * Parse the injected configuration, or throw.
 *
 * Deliberately fatal. A missing block means the page was served by something
 * other than Doppel's admin listener, and every default this could invent would
 * be wrong in a way the operator would have to debug: a made-up `public: false`
 * shows a token dialog on a public deployment, and a made-up header name sends
 * the token where nothing reads it.
 */
export function readRuntimeConfig(document_: Document = document): RuntimeConfig {
  const element = document_.getElementById(ELEMENT_ID)
  if (!element?.textContent) {
    throw new Error(
      `the page has no #${ELEMENT_ID} block; it was not served by Doppel's admin listener`,
    )
  }

  const parsed: unknown = JSON.parse(element.textContent)
  if (typeof parsed !== 'object' || parsed === null) {
    throw new Error(`#${ELEMENT_ID} is not an object: ${element.textContent}`)
  }
  return parsed as RuntimeConfig
}

/**
 * The configuration, read once.
 *
 * A function rather than a const so a test can re-read it after replacing the
 * document, and so the throw happens when something asks rather than at import.
 */
let cached: RuntimeConfig | undefined

export function runtimeConfig(): RuntimeConfig {
  cached ??= readRuntimeConfig()
  return cached
}

/** Test seam: forget the cached configuration. */
export function forgetRuntimeConfig(): void {
  cached = undefined
}
