// Checking a whole document, and reporting it in words.
//
// Against the real schema, like `rules.test.ts`, and for the same reason: a fixture
// would be a third copy of the rules. What is being tested here is not the validator
// -- that is a library -- but the reduction of its output to the mistakes, which is
// where the difference between "45 is greater than 1" and eleven lines of "a
// subschema had errors" lives.
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

import type { JsonSchema } from '../../types/schema'
import { proxyChecker } from '../document'

const schema = JSON.parse(
  readFileSync(join(__dirname, '../../../../doppel-config.schema.json'), 'utf8'),
) as JsonSchema

const check = proxyChecker(schema)!

/** The document `emptyMock` and the form's blank produce, which must be legal. */
const MINIMAL = { name: 'alpha', type: 'http', url: 'https://alpha.example.com/api/' }

describe('checking a proxy document', () => {
  it('is built for the proxy definition rather than the whole configuration', () => {
    expect(proxyChecker(schema)).toBeDefined()
    // Nothing to check against is a state the page survives: the server still
    // validates every save.
    expect(proxyChecker(undefined)).toBeUndefined()
    expect(proxyChecker({} as JsonSchema)).toBeUndefined()
  })

  it('accepts the documents the form produces', () => {
    expect(check(MINIMAL)).toEqual([])
    expect(
      check({
        ...MINIMAL,
        timeout: 30,
        body_limit: '4Mi',
        resolve: { type: 'header', header: 'X-Proxy-Name' },
        headers: { 'X-Injected': 'yes' },
        replace: 0.25,
        rewrite_redirects: false,
        loss: { percentage: 0.05, status: 503 },
        latency: { percentage: 0.5, min: 0.1, max: 1.5 },
        access: { read: 'public', update: ['admin'] },
        mocks: [
          {
            name: 'one-widget',
            request: {
              method: 'POST',
              url: '^/widgets$',
              body: { items: '.content.items' },
            },
            response: { status: 201, json: '{"ok": true}', headers: { 'X-Trace': '{{ id }}' } },
          },
        ],
      }),
    ).toEqual([])
  })

  it('names the field and the reason, once each', () => {
    const troubles = check({
      ...MINIMAL,
      name: 'has space',
      timeout: 5000,
      loss: { percentage: 45, status: 700 },
    })

    expect(troubles).toEqual(
      expect.arrayContaining([
        { where: 'name', what: 'string does not match pattern' },
        { where: 'timeout', what: '5000 is greater than 3600' },
        { where: 'loss.percentage', what: '45 is greater than 1' },
        { where: 'loss.status', what: '700 is greater than 599' },
      ]),
    )
    // And nothing else: no "a subschema had errors", no "expected null" from the
    // optional fields, no "does not match additional properties schema" for the four
    // fields that are perfectly well known.
    expect(troubles).toHaveLength(4)
  })

  it('calls an unknown field what it is', () => {
    const troubles = check({ ...MINIMAL, timeuot: 30 })
    expect(troubles).toEqual([{ where: 'timeuot', what: 'is not a field of this document' }])
  })

  it('reaches into a mock', () => {
    const troubles = check({
      ...MINIMAL,
      mocks: [
        {
          name: 'ok',
          request: { method: 'get', url: '^/$' },
          response: { status: 99 },
        },
      ],
    })
    expect(troubles).toEqual(
      expect.arrayContaining([
        { where: 'mocks.0.response.status', what: '99 is less than 100' },
      ]),
    )
    // Lower case `get` is refused by the enum, which is the mistake the reference
    // and the schema both warn about.
    expect(troubles.some((trouble) => trouble.where === 'mocks.0.request.method')).toBe(true)
  })

  it('says what is missing', () => {
    const troubles = check({ type: 'http' })
    const missing = troubles.filter((trouble) => trouble.what.includes('required'))
    expect(missing.length).toBeGreaterThan(0)
  })

  it('refuses something that is not a document at all', () => {
    // `load` returns a string for `hello`, which is not this function's business --
    // but a list is an object, and the caller passes it through.
    expect(check([]).length).toBeGreaterThan(0)
  })
})
