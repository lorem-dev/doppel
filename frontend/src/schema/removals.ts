// What a save would take away, in the words the form uses for it.
//
// A form full of Remove buttons has no undo, and the button that commits all of them
// says only "Save changes". So the save asks first, and this is what it asks about:
// the entries the document had when it was read and does not have now.
//
// Only removals. A changed value is visible in the field it is in -- the operator is
// looking at it -- while a removal leaves nothing behind to notice, which is exactly
// the edit worth confirming.

import type { MockConfig, ProxyConfig } from '../types/proxy'

/** The keys `before` had and `after` does not. */
function goneFrom(
  before: Record<string, string> | undefined,
  after: Record<string, string> | undefined,
): string[] {
  const kept = new Set(Object.keys(after ?? {}))
  return Object.keys(before ?? {}).filter((key) => !kept.has(key))
}

/** One mock's maps, by the name the form gives each. */
function mockMaps(mock: MockConfig): Array<[string, Record<string, string> | undefined]> {
  return [
    ['variables from headers', mock.request.headers],
    ['variables from the query', mock.request.query],
    ['variables from the body', mock.request.body],
    ['response headers', mock.response.headers],
  ]
}

/**
 * Everything a save would remove, one sentence each, in the order the form shows it.
 *
 * Named rather than counted: "2 mocks" is a number to check against a page the
 * operator has already scrolled past, while `one-widget` is the thing they either
 * meant to remove or did not.
 */
export function removals(before: ProxyConfig, after: ProxyConfig): string[] {
  const gone: string[] = []

  for (const key of goneFrom(before.headers, after.headers)) {
    gone.push(`the injected header \`${key}\``)
  }

  for (const action of ['read', 'update', 'delete', 'upload'] as const) {
    if (before.access?.[action] !== undefined && after.access?.[action] === undefined) {
      gone.push(`the \`${action}\` access override`)
    }
  }

  const kept = new Map((after.mocks ?? []).map((mock) => [mock.name, mock]))
  for (const mock of before.mocks ?? []) {
    const survivor = kept.get(mock.name)
    if (!survivor) {
      gone.push(`the mock \`${mock.name}\``)
      continue
    }
    // A mock that is still there can have lost parts of itself.
    for (const [what, map] of mockMaps(mock)) {
      const surviving = mockMaps(survivor).find(([name]) => name === what)?.[1]
      for (const key of goneFrom(map, surviving)) {
        gone.push(`\`${key}\` from ${what} of \`${mock.name}\``)
      }
    }
    if (mock.response.template !== undefined && survivor.response.template === undefined) {
      // The one removal the form makes without a Remove button: switching a mock's
      // answer away from its template file drops the name, and the name cannot be
      // typed back in -- templates are the API's, not this page's.
      gone.push(`the template file \`${mock.response.template}\` from \`${mock.name}\``)
    }
  }

  return gone
}
