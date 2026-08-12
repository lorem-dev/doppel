// The token, and whether to ask for one.
//
// The dialog is not a gate. `access.list` and `access.read` can be `public`, and
// a proxy's own `access` block can open one proxy to a group -- so the page has
// to be usable with no token at all, showing whatever the server is willing to
// show an anonymous caller.

import { create } from 'zustand'

import { clearToken, isRefused, loadToken, refuse, saveToken } from '../services/auth'
import { runtimeConfig } from '../services/runtimeConfig'

interface AuthState {
  token?: string
  /** Whether the token dialog is on screen. */
  dialogOpen: boolean
  /**
   * Open the dialog if there is something to ask for.
   *
   * Called by the shell on mount rather than decided when this module loads: a
   * store that read the page's configuration at import time would run before the
   * app had rendered anything, and would take every test that imports a component
   * with it.
   */
  askIfNeeded: () => void
  signIn: (token: string) => void
  signOut: () => void
  /** "Continue without a token": closes the dialog and remembers the refusal. */
  decline: () => void
  openDialog: () => void
  closeDialog: () => void
}

/**
 * Whether to open the dialog unprompted.
 *
 * Once, and only when there is something to ask for: a public deployment has no
 * tokens to present, a held token needs no dialog, and an operator who already
 * declined is not asked again for the rest of the tab.
 */
function shouldAsk(token: string | undefined): boolean {
  return !runtimeConfig().public && !token && !isRefused()
}

export const useAuth = create<AuthState>((set, get) => {
  return {
    token: loadToken(),
    dialogOpen: false,

    askIfNeeded: () => {
      if (shouldAsk(get().token)) {
        set({ dialogOpen: true })
      }
    },

    signIn: (value) => {
      saveToken(value)
      set({ token: value, dialogOpen: false })
    },

    // Signing out does not open the dialog. The page carries on anonymously and
    // shows what a caller with no token may see, which is the same state a first
    // visit to a partly-public deployment starts in.
    signOut: () => {
      clearToken()
      set({ token: undefined, dialogOpen: false })
    },

    decline: () => {
      refuse()
      set({ dialogOpen: false })
    },

    openDialog: () => {
      set({ dialogOpen: true })
    },
    closeDialog: () => {
      set({ dialogOpen: false })
    },
  }
})
