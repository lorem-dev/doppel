import { forgetRuntimeConfig } from '../../services/runtimeConfig'
import { saveToken } from '../../services/auth'

/** Serve the page with this configuration and re-import the store. */
async function withConfig(isPublic: boolean) {
  document.head.innerHTML = `<script type="application/json" id="doppel-config">${JSON.stringify({
    title: 'Doppel',
    public: isPublic,
    version: '0.4.1',
    authHeader: 'X-Proxy-Authorization',
    refreshMs: 60000,
  })}</script>`
  forgetRuntimeConfig()
  // The dialog's initial state is decided when the store is created, so the
  // module has to be fresh for each case.
  jest.resetModules()
  return import('../auth')
}

beforeEach(() => {
  localStorage.clear()
  sessionStorage.clear()
})

describe('the token dialog', () => {
  it('opens unprompted when a token is needed and none is held', async () => {
    const { useAuth } = await withConfig(false)
    expect(useAuth.getState().dialogOpen).toBe(false)
    // The shell asks on mount. Nothing is decided when the module loads, so a
    // component test that imports this store does not need a served page.
    useAuth.getState().askIfNeeded()
    expect(useAuth.getState().dialogOpen).toBe(true)
  })

  it('stays shut on a public deployment', async () => {
    // There are no tokens to present, so asking for one would be asking for
    // something that does not exist.
    const { useAuth } = await withConfig(true)
    useAuth.getState().askIfNeeded()
    expect(useAuth.getState().dialogOpen).toBe(false)
  })

  it('stays shut when a token is already held', async () => {
    saveToken('root-token')
    const { useAuth } = await withConfig(false)
    useAuth.getState().askIfNeeded()
    expect(useAuth.getState().dialogOpen).toBe(false)
    expect(useAuth.getState().token).toBe('root-token')
  })

  it('stays shut for the rest of the tab once declined', async () => {
    const first = await withConfig(false)
    first.useAuth.getState().askIfNeeded()
    first.useAuth.getState().decline()
    expect(first.useAuth.getState().dialogOpen).toBe(false)

    // A reload of the page, same tab: the refusal is remembered, so the operator
    // is not asked again.
    const second = await withConfig(false)
    second.useAuth.getState().askIfNeeded()
    expect(second.useAuth.getState().dialogOpen).toBe(false)
  })
})

describe('signing out', () => {
  it('drops the token and does not reopen the dialog', async () => {
    saveToken('root-token')
    const { useAuth } = await withConfig(false)

    useAuth.getState().signOut()
    expect(useAuth.getState().token).toBeUndefined()
    // The page carries on anonymously, showing whatever a caller with no token
    // may see. A blocking dialog would make "sign out" mean "leave".
    expect(useAuth.getState().dialogOpen).toBe(false)
    expect(localStorage.getItem('doppel.token')).toBeNull()
  })

  it('lets the dialog be opened again on purpose', async () => {
    const { useAuth } = await withConfig(false)
    useAuth.getState().decline()
    useAuth.getState().openDialog()
    expect(useAuth.getState().dialogOpen).toBe(true)
  })
})
