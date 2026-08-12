import { startTheme, useTheme } from '../theme'

/** A `matchMedia` that reports `dark` and can announce a change. */
function stubMatchMedia(dark: boolean): { announce: (dark: boolean) => void } {
  let matches = dark
  const listeners: Array<() => void> = []
  Object.defineProperty(window, 'matchMedia', {
    writable: true,
    configurable: true,
    value: () => ({
      get matches() {
        return matches
      },
      addEventListener: (_: string, listener: () => void) => listeners.push(listener),
      removeEventListener: () => {},
    }),
  })
  return {
    announce: (next) => {
      matches = next
      for (const listener of listeners) {
        listener()
      }
    },
  }
}

beforeEach(() => {
  localStorage.clear()
  document.documentElement.className = ''
})

describe('the theme', () => {
  it('follows the system when nothing was chosen', () => {
    // The default is `system`, and `system` is not a synonym for light: an
    // operator with a dark desktop should not be handed the one window that
    // glows.
    stubMatchMedia(true)
    useTheme.setState({ choice: 'system', resolved: 'dark' })
    startTheme()

    expect(useTheme.getState().choice).toBe('system')
    expect(document.documentElement.classList.contains('dark')).toBe(true)
  })

  it('keeps following the system while it changes', () => {
    const media = stubMatchMedia(false)
    useTheme.setState({ choice: 'system', resolved: 'light' })
    startTheme()
    expect(document.documentElement.classList.contains('dark')).toBe(false)

    media.announce(true)
    expect(useTheme.getState().resolved).toBe('dark')
    expect(document.documentElement.classList.contains('dark')).toBe(true)
  })

  it('stops following the system once a choice is made', () => {
    const media = stubMatchMedia(false)
    startTheme()
    useTheme.getState().set('light')

    // A page someone set to light must not go dark because their desktop did.
    media.announce(true)
    expect(useTheme.getState().resolved).toBe('light')
    expect(document.documentElement.classList.contains('dark')).toBe(false)
  })

  it('remembers the choice across a reload', () => {
    stubMatchMedia(false)
    useTheme.getState().set('dark')
    expect(localStorage.getItem('doppel.theme')).toBe('dark')
  })

  it('survives a browser with no matchMedia at all', () => {
    // Not hypothetical: jsdom is such a browser, and a store that threw on
    // import would take every suite with it.
    Object.defineProperty(window, 'matchMedia', {
      writable: true,
      configurable: true,
      value: undefined,
    })
    expect(() => startTheme()).not.toThrow()
  })
})
