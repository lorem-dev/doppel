// What the save dialog will list.
//
// The interesting cases are the two kinds of quiet loss: a mock that is gone, and a
// mock that is still there minus one of its entries. Both look identical on a page
// scrolled past the section they were in.

import type { ProxyConfig } from '../../types/proxy'
import { removals } from '../removals'

const BASE: ProxyConfig = {
  name: 'alpha',
  type: 'http',
  url: 'https://alpha.example.com/api/',
  headers: { 'X-Injected': 'yes', 'X-Other': 'no' },
  access: { read: 'public', update: ['admin'] },
  mocks: [
    {
      name: 'one-widget',
      request: {
        method: 'GET',
        url: '^/widgets$',
        headers: { trace: 'X-Trace-Id' },
        query: { filter: '.filter' },
      },
      response: { status: 200, body: 'ok', headers: { 'X-Mocked': 'one-widget' } },
    },
    {
      name: 'two-widgets',
      request: { method: 'GET', url: '^/widgets/2$' },
      response: { status: 200, template: 'two.json.j2' },
    },
  ],
}

/**
 * `BASE` with one edit applied, so each case says only what it changes.
 *
 * Cloned through JSON rather than with `structuredClone`, which jsdom does not have.
 * A proxy document is plain data by construction -- it goes over the wire as JSON --
 * so the two are the same thing here.
 */
function edited(edit: (draft: ProxyConfig) => void): ProxyConfig {
  const draft = JSON.parse(JSON.stringify(BASE)) as ProxyConfig
  edit(draft)
  return draft
}

describe('what a save would remove', () => {
  it('says nothing about a document that lost nothing', () => {
    expect(removals(BASE, BASE)).toEqual([])
    // A changed value is not a removal: it is visible in the field the operator is
    // looking at.
    expect(removals(BASE, edited((draft) => (draft.url = 'https://beta.example.com/')))).toEqual([])
    // Nor is something added.
    expect(
      removals(
        BASE,
        edited((draft) => {
          draft.headers = { ...draft.headers, 'X-New': 'yes' }
        }),
      ),
    ).toEqual([])
  })

  it('names a mock that is gone', () => {
    const gone = removals(
      BASE,
      edited((draft) => {
        draft.mocks = draft.mocks?.filter((mock) => mock.name !== 'one-widget')
      }),
    )
    expect(gone).toContain('the mock `one-widget`')
    // And nothing about the mock's own entries, which would be the same loss twice.
    expect(gone).toHaveLength(1)
  })

  it('names an entry a surviving mock lost', () => {
    const gone = removals(
      BASE,
      edited((draft) => {
        delete draft.mocks![0]!.request.headers
        delete draft.mocks![0]!.response.headers!['X-Mocked']
      }),
    )
    expect(gone).toEqual([
      '`trace` from variables from headers of `one-widget`',
      '`X-Mocked` from response headers of `one-widget`',
    ])
  })

  it('names an injected header and an access override', () => {
    const gone = removals(
      BASE,
      edited((draft) => {
        delete draft.headers!['X-Other']
        delete draft.access!.update
      }),
    )
    expect(gone).toEqual(['the injected header `X-Other`', 'the `update` access override'])
  })

  it('names a template file the answer was switched away from', () => {
    // The one loss the form makes without a Remove button, and the one that cannot be
    // undone by typing: the dashboard offers no way to name a template file.
    const gone = removals(
      BASE,
      edited((draft) => {
        draft.mocks![1]!.response = { status: 200, body: '' }
      }),
    )
    expect(gone).toEqual(['the template file `two.json.j2` from `two-widgets`'])
  })

  it('reports every loss when a document is emptied out', () => {
    const gone = removals(BASE, { name: 'alpha', type: 'http', url: BASE.url })
    expect(gone).toEqual([
      'the injected header `X-Injected`',
      'the injected header `X-Other`',
      'the `read` access override',
      'the `update` access override',
      'the mock `one-widget`',
      'the mock `two-widgets`',
    ])
  })
})
