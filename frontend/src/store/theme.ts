// Light, dark, or whatever the operating system says.
//
// `system` is the default, and it is not the same as "light": an operator whose
// desktop is dark expects a dark page without having chosen anything, and a
// dashboard that ignores that is the one window that glows.

import { create } from 'zustand'

export type ThemeChoice = 'light' | 'dark' | 'system'
export type Resolved = 'light' | 'dark'

const STORAGE_KEY = 'doppel.theme'
const DARK_QUERY = '(prefers-color-scheme: dark)'

/** What the operating system says, or `light` where nothing says anything. */
function systemPrefers(): Resolved {
  // Optional call rather than a bare one: jsdom has no `matchMedia`, and a store
  // that threw on import would take every test with it.
  return window.matchMedia?.(DARK_QUERY).matches ? 'dark' : 'light'
}

function stored(): ThemeChoice {
  const raw = localStorage.getItem(STORAGE_KEY)
  return raw === 'light' || raw === 'dark' || raw === 'system' ? raw : 'system'
}

function resolve(choice: ThemeChoice): Resolved {
  return choice === 'system' ? systemPrefers() : choice
}

/**
 * Put the resolved theme on `<html>`.
 *
 * A class rather than a data attribute, because that is what the `dark:` variant
 * in `index.css` is defined against.
 */
function apply(resolved: Resolved): void {
  document.documentElement.classList.toggle('dark', resolved === 'dark')
}

interface ThemeState {
  choice: ThemeChoice
  resolved: Resolved
  set: (choice: ThemeChoice) => void
  /** Re-resolve after the system preference changed. */
  systemChanged: () => void
}

export const useTheme = create<ThemeState>((set, get) => ({
  choice: stored(),
  resolved: resolve(stored()),

  set: (choice) => {
    localStorage.setItem(STORAGE_KEY, choice)
    const resolved = resolve(choice)
    apply(resolved)
    set({ choice, resolved })
  },

  systemChanged: () => {
    // Only `system` follows the system. A page someone set to light must not go
    // dark at sunset because their desktop did.
    if (get().choice !== 'system') {
      return
    }
    const resolved = systemPrefers()
    apply(resolved)
    set({ resolved })
  },
}))

/**
 * Apply the stored choice and follow the system while it is `system`.
 *
 * Called once from the app rather than at import: a module that touched the
 * document on load would run in every test that imports anything near it.
 */
export function startTheme(): () => void {
  apply(useTheme.getState().resolved)

  const query = window.matchMedia?.(DARK_QUERY)
  if (!query) {
    return () => {}
  }
  const listener = () => {
    useTheme.getState().systemChanged()
  }
  query.addEventListener('change', listener)
  return () => {
    query.removeEventListener('change', listener)
  }
}
