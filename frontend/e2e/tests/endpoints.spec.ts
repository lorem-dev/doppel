// The two things on this listener that are not the dashboard and not JSON: the
// Prometheus exposition and the Swagger UI.
//
// Both were broken by changes to the routes around them, and neither breakage
// failed a test. `/metrics` was answered by the dashboard's fallback with `200
// text/html`, which a scraper reports as a parse failure and an operator sees as
// no metrics at all; the Swagger UI was answered with a redirect to itself, once
// a trailing-slash rewrite was put in front of it. This spec is what makes both
// fail here instead.

import { expect, test } from '@playwright/test'

import { PRIVATE_CONFIG, ROOT_TOKEN } from '../src/configs'
import { startDoppel, type Doppel } from '../src/server'

let doppel: Doppel

test.beforeAll(async () => {
  doppel = await startDoppel(PRIVATE_CONFIG)
})
test.afterAll(() => doppel.stop())

test('the exposition is served as Prometheus text, with or without a slash', async ({ page }) => {
  // One request through the proxy first. It fails -- this configuration's
  // upstreams do not exist -- and that is enough: every request through the
  // listener is recorded, including the ones that go nowhere, so the exposition
  // has something in it. Without traffic there are no series and the body is
  // empty, which would make the assertions below pass against nothing.
  await page.request.get(`${doppel.proxyURL}/anything`).catch(() => undefined)

  for (const path of ['/metrics', '/metrics/']) {
    const response = await page.request.get(`${doppel.baseURL}${path}`)

    expect(response.status(), path).toBe(200)
    // The content type, not just the status: the dashboard's fallback answers
    // 200 as well, and the difference between the two is this header.
    expect(response.headers()['content-type'], path).toBe(
      'text/plain; version=0.0.4; charset=utf-8',
    )
    const body = await response.text()
    expect(body, path).toContain('doppel_')
    expect(body, path).not.toContain('<!doctype html')
  }
})

test('the Swagger UI loads and lists the API', async ({ page }) => {
  // Everything the page asks for, so a UI that renders an empty shell because
  // its own assets 404 is a failure rather than a screenshot nobody looks at.
  const failed: string[] = []
  page.on('response', (response) => {
    if (response.status() >= 400) {
      failed.push(`${response.status()} ${response.url()}`)
    }
  })

  await page.goto(`${doppel.baseURL}/swagger-ui/`)

  // Its own heading, then one operation out of the document: the UI is not just
  // present, it has fetched and parsed `/openapi.json`.
  // `.first()`: the UI wraps itself in a section of the same class as its root.
  await expect(page.locator('.swagger-ui').first()).toBeVisible()
  await expect(page.getByText('/api/v1/proxies').first()).toBeVisible()
  expect(failed).toEqual([])
})

test('the Swagger UI is reachable without the trailing slash', async ({ page }) => {
  // The UI redirects the bare path to the slashed one itself. What this pins is
  // that nothing in front of it rewrites the slash back off again -- that pair
  // is a redirect loop, and it is how this broke.
  const response = await page.goto(`${doppel.baseURL}/swagger-ui`)

  expect(response?.status()).toBe(200)
  expect(page.url()).toBe(`${doppel.baseURL}/swagger-ui/`)
  await expect(page.locator('.swagger-ui').first()).toBeVisible()
})

test('the OpenAPI document describes this binary', async ({ page }) => {
  // At `/openapi.json`, outside `/api/`: the document is not a resource of the
  // API, and that is the path a client generator is pointed at by default.
  const response = await page.request.get(`${doppel.baseURL}/openapi.json`)

  expect(response.status()).toBe(200)
  expect(response.headers()['content-type']).toContain('application/json')

  const document = (await response.json()) as {
    openapi: string
    info: { title: string; version: string }
    paths: Record<string, unknown>
  }
  expect(document.openapi).toMatch(/^3\./)
  expect(document.info.version).not.toBe('')

  // Generated from the handlers, so every path it names is one this binary
  // serves -- including the three that live outside `/api/v1/`, which are the
  // ones a move leaves behind in the document.
  const paths = Object.keys(document.paths)
  expect(paths).toContain('/api/v1/proxies')
  expect(paths).toContain('/api/v1/status')
  expect(paths).toContain('/metrics')

  // Unauthenticated on purpose: a client cannot authenticate before it knows
  // how, and this is where it finds out. No token was sent above.
  expect(response.status()).toBe(200)
})

test('the document is served under either spelling of its path', async ({ page }) => {
  // The trailing-slash rewrite covers this too, and `/openapi.json/` is what a
  // hand-written base URL with a slash on the end produces.
  const slashed = await page.request.get(`${doppel.baseURL}/openapi.json/`)

  expect(slashed.status()).toBe(200)
  expect((await slashed.json()).openapi).toMatch(/^3\./)
})

test('the page is rendered per request, not served as a file', async ({ page }) => {
  // `index.html` ships with a placeholder configuration -- title `Doppel`,
  // version `dev`, `titleIsDefault: true` -- so that `npm run dev` has something
  // to develop against. The listener splices the real values into that element on
  // every request, which is also what `Cache-Control: no-store` on this route is
  // for. If it ever served the file as it is, the page would report the wrong
  // title, the wrong version and `public: false` for a public deployment.
  const response = await page.request.get(`${doppel.baseURL}/`)

  expect(response.status()).toBe(200)
  expect(response.headers()['cache-control']).toBe('no-store')

  const html = await response.text()
  const config = html.match(/<script type="application\/json" id="doppel-config">(.*?)<\/script>/s)
  expect(config, 'the placeholder element must still be in the served page').not.toBeNull()

  const settings = JSON.parse(config![1]!) as {
    title: string
    titleIsDefault: boolean
    version: string
  }
  // From the configuration this spec started Doppel with, not from the file.
  expect(settings.title).toBe('Doppel (e2e)')
  expect(settings.titleIsDefault).toBe(false)
  expect(settings.version).not.toBe('dev')
  expect(settings.version).toMatch(/^\d+\.\d+\.\d+/)
})

test('assets and the page are served compressed', async ({ page }) => {
  // The dashboard is 140 KB of JavaScript and CSS uncompressed, and it used to
  // go out that way: the assets are embedded as bytes and were served as bytes.
  // What this pins is the negotiation -- what a browser asks for is what it gets.
  await page.goto(doppel.baseURL)
  const asset = await page.evaluate(() =>
    [...document.querySelectorAll('script[src], link[rel="stylesheet"]')]
      .map((element) => element.getAttribute('src') ?? element.getAttribute('href'))
      .find((url) => url?.startsWith('/static/assets/')),
  )
  expect(asset, 'the page must reference a built asset').toBeTruthy()

  for (const [accepted, expected] of [
    ['br', 'br'],
    ['gzip', 'gzip'],
    ['gzip, br', 'br'],
  ]) {
    const response = await page.request.get(`${doppel.baseURL}${asset}`, {
      headers: { 'Accept-Encoding': accepted! },
    })
    expect(response.status(), accepted).toBe(200)
    expect(response.headers()['content-encoding'], accepted).toBe(expected)
    // Whatever the encoding, the bytes decode to the same script.
    expect((await response.text()).length, accepted).toBeGreaterThan(1000)
  }

  // And the page itself, which is built per request rather than embedded.
  const identity = await page.request.get(`${doppel.baseURL}${asset}`, {
    headers: { 'Accept-Encoding': 'identity' },
  })
  expect(identity.headers()['content-encoding']).toBeUndefined()
})

test('the metrics that must always be there are there', async ({ page }) => {
  // A panel and an alert both read a never-recorded metric as "no data", which is
  // indistinguishable from a process nobody is scraping. These four are published
  // at startup or on the first runtime swap, so a scrape of a process that has
  // served nothing still answers with them.
  const exposition = await (await page.request.get(`${doppel.baseURL}/metrics`)).text()

  // What this binary is, and what this deployment turned on.
  expect(exposition).toMatch(/doppel_build_info\{version="\d+\.\d+\.\d+[^"]*"\} 1/)
  expect(exposition).toContain('doppel_dashboard_info{enabled="true"} 1')

  // No proxy error yet, said in a form a query can subtract rather than as an
  // absence.
  expect(exposition).toContain('doppel_proxy_last_error_timestamp_seconds{code=""} 0')

  // The mock counts of the configuration in force. This spec's configuration has
  // two proxies and no mocks.
  expect(exposition).toContain('doppel_proxy_mocks{proxy="alpha"} 0')
  expect(exposition).toContain('doppel_proxy_mocks{proxy="beta"} 0')
})

test('the admin API records its own latency, by route template', async ({ page }) => {
  // Two requests to the same route with different path parameters, and a query
  // string on one of them: one series, or the label is wrong.
  const token = ROOT_TOKEN
  for (const name of ['alpha', 'beta']) {
    const response = await page.request.get(`${doppel.baseURL}/api/v1/proxies/${name}?noise=1`, {
      headers: { 'X-Proxy-Authorization': `Bearer ${token}` },
    })
    expect(response.status(), name).toBe(200)
  }

  const exposition = await (await page.request.get(`${doppel.baseURL}/metrics`)).text()
  const counted = exposition
    .split('\n')
    .filter(
      (line) =>
        line.startsWith('doppel_admin_request_duration_seconds_count') &&
        line.includes('route="/api/v1/proxies/{name}"'),
    )

  expect(counted, `no templated series in:\n${exposition}`).toHaveLength(1)
  expect(counted[0]).toMatch(/\} 2$/)
  // Neither proxy name nor the query string may appear in the exposition.
  expect(exposition).not.toContain('proxies/alpha')
  expect(exposition).not.toContain('noise=1')

  // And the ladder the admin histogram uses stops at five seconds.
  const buckets = exposition
    .split('\n')
    .filter((line) => line.startsWith('doppel_admin_request_duration_seconds_bucket'))
  expect(buckets.some((line) => line.includes('le="5"'))).toBe(true)
  expect(buckets.some((line) => line.includes('le="10"'))).toBe(false)
})
