// The token, in the browser.
//
// `localStorage` with a one-hour lifetime enforced on read. Two things this is
// not, both worth saying plainly because the opposite is easy to assume:
//
//   - It is not a session. The token does not expire server-side; forgetting it
//     here only means this browser stops presenting it. Revoking a token is a
//     configuration change.
//   - It is not a security boundary. Anything running in this page can read
//     `localStorage`. The lifetime exists so an unattended browser stops holding
//     a working admin token indefinitely, which is a different and smaller
//     claim.

const TOKEN_KEY = 'doppel.token'
const REFUSED_KEY = 'doppel.token.refused'

/** One hour, in milliseconds. */
export const TOKEN_LIFETIME_MS = 60 * 60 * 1000

interface StoredToken {
  token: string
  /** When it was entered, as epoch milliseconds. */
  savedAt: number
}

/**
 * The stored token, or `undefined` if there is none or it has aged out.
 *
 * An expired entry is deleted here rather than left to be cleaned up later: the
 * next read would otherwise have to make the same judgement again, and one of
 * the two callers would eventually forget.
 */
export function loadToken(now: number = Date.now()): string | undefined {
  const raw = localStorage.getItem(TOKEN_KEY)
  if (!raw) {
    return undefined
  }

  let stored: StoredToken
  try {
    stored = JSON.parse(raw) as StoredToken
  } catch {
    // Written by an older version, or by hand. Unreadable is the same as absent,
    // and leaving it would make every later read fail the same way.
    localStorage.removeItem(TOKEN_KEY)
    return undefined
  }

  if (typeof stored.token !== 'string' || typeof stored.savedAt !== 'number') {
    localStorage.removeItem(TOKEN_KEY)
    return undefined
  }
  if (now - stored.savedAt >= TOKEN_LIFETIME_MS) {
    localStorage.removeItem(TOKEN_KEY)
    return undefined
  }
  return stored.token
}

export function saveToken(token: string, now: number = Date.now()): void {
  const stored: StoredToken = { token, savedAt: now }
  localStorage.setItem(TOKEN_KEY, JSON.stringify(stored))
  // Entering a token withdraws an earlier refusal, so signing out later opens
  // the dialog again rather than staying silent.
  sessionStorage.removeItem(REFUSED_KEY)
}

export function clearToken(): void {
  localStorage.removeItem(TOKEN_KEY)
}

/**
 * Whether the operator has already declined to enter a token.
 *
 * In `sessionStorage`, not `localStorage`: the refusal is about this sitting at
 * this tab, and remembering it forever would mean an operator who once said "not
 * now" is never offered the dialog again.
 */
export function isRefused(): boolean {
  return sessionStorage.getItem(REFUSED_KEY) === 'yes'
}

export function refuse(): void {
  sessionStorage.setItem(REFUSED_KEY, 'yes')
}
