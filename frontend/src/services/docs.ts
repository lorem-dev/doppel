// Where the documentation is, for this version of Doppel.
//
// Every link out of the dashboard carries the running version. The site is
// published per version -- `.../doppel/0.4.1/...`, with `latest` and `dev` as
// aliases -- so a page served by 0.4.1 that linked to the current site would show
// whoever followed it the rules of whatever has been released since. The version
// comes from the injected configuration, which is the process's own.

import { runtimeConfig } from './runtimeConfig'

/** The published site, without a version. */
const SITE = 'https://lorem-dev.github.io/doppel'

/** The repository, which is not versioned and does not pretend to be. */
export const REPOSITORY = 'https://github.com/lorem-dev/doppel'

/** The documentation root for the running version, with its trailing slash. */
export function docsRoot(): string {
  return `${SITE}/${runtimeConfig().version}/`
}

/** A page of the documentation, by its path under the site root. */
export function docsUrl(page: string): string {
  return `${docsRoot()}${page}`
}

/**
 * The anchor a configuration parameter is documented under.
 *
 * `mocks[].request.url` becomes `proxies-mocks-request-url`: brackets go, dots
 * become dashes. `scripts/parameters_doc.py` writes exactly these anchors into
 * `usage/parameters.md`, and `scripts/check_docs_links.py` fails the build if a path
 * used here has no section there -- which is the only way an (i) that goes nowhere
 * gets caught, since nothing in a browser reports a bad fragment.
 */
export function parameterAnchor(path: string): string {
  // The form edits one proxy, so its paths are relative to a proxy document while
  // the reference documents a whole configuration.
  return `proxies.${path}`.replace(/\[\]/g, '').replace(/\./g, '-')
}

/** The section of the parameter reference for one field of a proxy. */
export function parameterUrl(path: string): string {
  return `${docsUrl('usage/parameters/')}#${parameterAnchor(path)}`
}
