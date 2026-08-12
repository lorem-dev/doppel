// The shell's components, minus the ones that need a router.
//
// react-router ships ESM only and reaches for `import.meta`, which jest's CJS
// transform cannot parse -- so anything rendering a NavLink or calling
// useNavigate is covered by the Playwright suite instead, where a real browser
// runs the real router. That is a better test of navigation anyway; what is left
// here is the part that is genuinely a unit.
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'

import { Footer } from '../Footer'
import { TokenDialog } from '../TokenDialog'
import { forgetRuntimeConfig } from '../../services/runtimeConfig'
import { useAuth } from '../../store/auth'

/**
 * Serve the page with this configuration.
 *
 * No module reset: nothing reads the configuration at import time any more, and
 * resetting would hand the test a different store instance than the one the
 * rendered component is subscribed to -- which looks like a component that
 * ignores its own state.
 */
function serve(overrides: { public?: boolean; title?: string; version?: string } = {}): void {
  document.head.innerHTML = `<script type="application/json" id="doppel-config">${JSON.stringify({
    title: overrides.title ?? 'Doppel',
    public: overrides.public ?? false,
    version: overrides.version ?? '0.4.1',
    authHeader: 'X-Proxy-Authorization',
    refreshMs: 60000,
  })}</script>`
  forgetRuntimeConfig()
}

beforeEach(() => {
  localStorage.clear()
  sessionStorage.clear()
})

describe('the footer', () => {
  it('shows the copyright and the version of the binary serving the page', () => {
    // From the injected configuration, not from package.json: what matters is
    // which Doppel answered.
    serve({ version: '9.9.9' })
    render(<Footer />)

    expect(screen.getByText(/\(c\) 2026 Lorem Dev/)).toBeInTheDocument()
    expect(screen.getByText(/Doppel 9\.9\.9/)).toBeInTheDocument()
  })
})

describe('the token dialog', () => {
  it('takes no for an answer', async () => {
    // `list` and `read` can be public, so a caller with no token may have plenty
    // to look at. A dialog that could not be dismissed would hide a working page.
    serve()
    useAuth.setState({ dialogOpen: true, token: undefined })
    render(<TokenDialog />)

    await userEvent.click(screen.getByRole('button', { name: 'Continue without a token' }))
    expect(useAuth.getState().dialogOpen).toBe(false)
    expect(sessionStorage.getItem('doppel.token.refused')).toBe('yes')
  })

  it('keeps the token out of the DOM as text', async () => {
    serve()
    useAuth.setState({ dialogOpen: true, token: undefined })
    render(<TokenDialog />)

    const input = screen.getByLabelText('Token')
    // A password input, so a shoulder or a screenshot does not carry an admin
    // token away.
    expect(input).toHaveAttribute('type', 'password')

    await userEvent.type(input, 'root-token-0000000000000000000000000')
    await userEvent.click(screen.getByRole('button', { name: 'Use this token' }))
    expect(useAuth.getState().token).toBe('root-token-0000000000000000000000000')
  })

  it('is not rendered at all when it is shut', () => {
    serve()
    useAuth.setState({ dialogOpen: false })
    const { container } = render(<TokenDialog />)
    expect(container).toBeEmptyDOMElement()
  })
})
