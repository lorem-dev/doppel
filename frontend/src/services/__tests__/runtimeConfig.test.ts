import { forgetRuntimeConfig, readRuntimeConfig } from '../runtimeConfig'

/**
 * A page carrying `contents` in the configuration element, escaped the way the
 * server escapes it.
 *
 * The escape is not incidental to the fixture -- it is the thing under test. An
 * unescaped less-than closes the script element early, which is why the server
 * replaces it, and a fixture that skipped the escape would be testing a page the
 * server never serves.
 */
const page = (contents: string): Document => {
  const escaped = contents.replace(/</g, '\\u003c')
  const parsed = new DOMParser().parseFromString(
    `<html><head><script type="application/json" id="doppel-config">${escaped}</script></head><body></body></html>`,
    'text/html',
  )
  return parsed
}

afterEach(forgetRuntimeConfig)

describe('the injected configuration', () => {
  it('is read from the page', () => {
    const config = readRuntimeConfig(
      page(
        '{"title":"Doppel (staging)","titleIsDefault":false,"public":true,"version":"0.4.1","authHeader":"X-Admin","refreshMs":60000,"copyrightYear":2026}',
      ),
    )
    expect(config.title).toBe('Doppel (staging)')
    expect(config.titleIsDefault).toBe(false)
    expect(config.public).toBe(true)
    expect(config.version).toBe('0.4.1')
    expect(config.authHeader).toBe('X-Admin')
    expect(config.refreshMs).toBe(60000)
    expect(config.copyrightYear).toBe(2026)
  })

  it('carries a title containing markup through intact', () => {
    // The server escapes the less-than so the element cannot be closed early;
    // what arrives here is the operator's title, exactly as they wrote it.
    const hostile = '</script><script>alert(1)</script>'
    const config = readRuntimeConfig(page(JSON.stringify({ title: hostile })))
    expect(config.title).toBe(hostile)
  })

  it('throws when the block is missing rather than inventing defaults', () => {
    // Every default this could invent is wrong in a way somebody would have to
    // debug: a made-up `public: false` shows a pointless dialog, and a made-up
    // header name sends the token where nothing reads it.
    const empty = new DOMParser().parseFromString('<html><body></body></html>', 'text/html')
    expect(() => readRuntimeConfig(empty)).toThrow(/doppel-config/)
  })

  it('throws when the block is not an object', () => {
    expect(() => readRuntimeConfig(page('"a string"'))).toThrow(/not an object/)
  })
})
