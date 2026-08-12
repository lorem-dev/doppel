// Where the dashboard sends a reader, and with which version in the URL.
//
// The anchors are checked against the generated reference by
// `scripts/check_docs_links.py`; what is checked here is the rule that builds them,
// because the script and this file have to agree on it.

import { docsRoot, docsUrl, parameterAnchor, parameterUrl } from '../docs'
import { forgetRuntimeConfig } from '../runtimeConfig'

function serve(version: string): void {
  document.head.innerHTML = `<script type="application/json" id="doppel-config">${JSON.stringify(
    {
      title: 'Doppel',
      titleIsDefault: true,
      public: false,
      version,
      authHeader: 'X-Proxy-Authorization',
      refreshMs: 60000,
      copyrightYear: 2026,
    },
  )}</script>`
  forgetRuntimeConfig()
}

afterEach(forgetRuntimeConfig)

describe('the documentation link', () => {
  it('carries the version of the binary serving the page', () => {
    // The site is published per version. A link to the unversioned root would show
    // whoever followed it the rules of whatever has been released since.
    serve('0.4.1')
    expect(docsRoot()).toBe('https://lorem-dev.github.io/doppel/0.4.1/')
    expect(docsUrl('usage/parameters/')).toBe(
      'https://lorem-dev.github.io/doppel/0.4.1/usage/parameters/',
    )

    serve('9.9.9')
    expect(docsRoot()).toBe('https://lorem-dev.github.io/doppel/9.9.9/')
  })
})

describe('a parameter anchor', () => {
  it('is the field path with the brackets gone and the dots turned to dashes', () => {
    // The same rule `scripts/parameters_doc.py` writes into the page, and the
    // script that compares the two spells it out for exactly this reason.
    expect(parameterAnchor('name')).toBe('proxies-name')
    expect(parameterAnchor('loss.percentage')).toBe('proxies-loss-percentage')
    expect(parameterAnchor('mocks[].request.url')).toBe('proxies-mocks-request-url')
    expect(parameterAnchor('mocks[].response.headers')).toBe('proxies-mocks-response-headers')
  })

  it('lands on the parameter reference for the running version', () => {
    serve('0.4.1')
    expect(parameterUrl('timeout')).toBe(
      'https://lorem-dev.github.io/doppel/0.4.1/usage/parameters/#proxies-timeout',
    )
  })
})
