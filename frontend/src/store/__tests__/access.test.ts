import type { AccessReport } from '../../types/api'
import { permits } from '../access'

const report = (overrides: Partial<AccessReport> = {}): AccessReport => ({
  caller: { kind: 'token', name: 'reader', group: 'user' },
  global: { list: true, read: true, create: false, update: false, delete: false, upload: false },
  ...overrides,
})

describe('permits()', () => {
  it('refuses everything until the report arrives', () => {
    // Assuming permitted and finding out on click is the button-that-does-nothing
    // this endpoint exists to prevent. A briefly disabled control is the smaller
    // cost.
    expect(permits(undefined, 'list')).toBe(false)
    expect(permits(undefined, 'create')).toBe(false)
  })

  it('answers from the global block', () => {
    const only = report()
    expect(permits(only, 'read')).toBe(true)
    expect(permits(only, 'create')).toBe(false)
  })

  it('prefers a proxy override over the global answer', () => {
    // The precedence the server applies, and the reason the map exists at all.
    const overridden = report({
        global: {
          list: true,
          read: false,
          create: false,
          update: false,
          delete: false,
          upload: false,
        },
        proxies: {
          alpha: { read: true, update: false, delete: false, upload: false },
          beta: { read: false, update: false, delete: false, upload: false },
        },
    })

    expect(permits(overridden, 'read', 'alpha')).toBe(true)
    expect(permits(overridden, 'read', 'beta')).toBe(false)
    expect(permits(overridden, 'read')).toBe(false)
  })

  it('falls back to the global answer for an unlisted proxy', () => {
    const mapped = report({
      proxies: { alpha: { read: true, update: false, delete: false, upload: false } },
    })
    expect(permits(mapped, 'read', 'gamma')).toBe(true)
  })

  it('ignores the map for actions a proxy cannot override', () => {
    // `list` and `create` are not about one proxy, and `ProxyAccessConfig` cannot
    // spell them -- so passing a proxy name must not change the answer.
    const mapped = report({
      proxies: { alpha: { read: true, update: false, delete: false, upload: false } },
    })
    expect(permits(mapped, 'create', 'alpha')).toBe(false)
  })
})
