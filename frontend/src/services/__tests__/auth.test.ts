import { TOKEN_LIFETIME_MS, clearToken, isRefused, loadToken, refuse, saveToken } from '../auth'

beforeEach(() => {
  localStorage.clear()
  sessionStorage.clear()
})

describe('the stored token', () => {
  it('comes back until it ages out', () => {
    const entered = 1_000_000
    saveToken('root-token', entered)

    expect(loadToken(entered)).toBe('root-token')
    expect(loadToken(entered + TOKEN_LIFETIME_MS - 1)).toBe('root-token')
    // The boundary is the interesting case: at exactly an hour it is gone, so
    // "expires after an hour" is not a claim that quietly means "after an hour
    // and a bit".
    expect(loadToken(entered + TOKEN_LIFETIME_MS)).toBeUndefined()
  })

  it('is erased when it ages out, not merely hidden', () => {
    saveToken('root-token', 0)
    expect(loadToken(TOKEN_LIFETIME_MS)).toBeUndefined()
    // Left behind, the next read would make the same judgement again -- and a
    // stale admin token sitting in storage is exactly what the lifetime is for.
    expect(localStorage.getItem('doppel.token')).toBeNull()
  })

  it('treats an unreadable entry as absent and clears it', () => {
    localStorage.setItem('doppel.token', 'not json')
    expect(loadToken()).toBeUndefined()
    expect(localStorage.getItem('doppel.token')).toBeNull()

    localStorage.setItem('doppel.token', JSON.stringify({ token: 42 }))
    expect(loadToken()).toBeUndefined()
  })

  it('is gone after signing out', () => {
    saveToken('root-token')
    clearToken()
    expect(loadToken()).toBeUndefined()
  })
})

describe('declining the dialog', () => {
  it('is remembered for the tab and withdrawn by entering a token', () => {
    expect(isRefused()).toBe(false)
    refuse()
    expect(isRefused()).toBe(true)

    // Entering a token means the operator is no longer declining, so signing out
    // later has to offer the dialog again rather than staying silent.
    saveToken('root-token')
    expect(isRefused()).toBe(false)
  })

  it('is kept out of localStorage, so it does not outlive the tab', () => {
    refuse()
    expect(localStorage.getItem('doppel.token.refused')).toBeNull()
    expect(sessionStorage.getItem('doppel.token.refused')).toBe('yes')
  })
})
