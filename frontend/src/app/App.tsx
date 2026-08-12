import { Suspense, useEffect } from 'react'
import { Outlet } from 'react-router'

import { Footer } from '../components/Footer'
import { Header } from '../components/Header'
import { Spinner } from '../components/Spinner'
import { Toasts } from '../components/Toasts'
import { TokenDialog } from '../components/TokenDialog'
import { runtimeConfig } from '../services/runtimeConfig'
import { useAccess } from '../store/access'
import { useAuth } from '../store/auth'
import { useSchema } from '../store/schema'
import { startTheme } from '../store/theme'

/**
 * The shell every page is drawn inside.
 *
 * It owns three things no page should own: the theme's lifetime, refetching the
 * caller's rights whenever the token changes, and fetching the configuration
 * schema. Rights are per-token, so a page that fetched them for itself would fetch
 * them again on every navigation and still miss a sign-out that happened elsewhere.
 * The schema is the same document for the life of the process, so it is fetched
 * once here rather than per form.
 */
export function App() {
  const token = useAuth((state) => state.token)
  const askIfNeeded = useAuth((state) => state.askIfNeeded)
  const refreshAccess = useAccess((state) => state.refresh)
  const loadSchema = useSchema((state) => state.load)

  useEffect(() => startTheme(), [])

  useEffect(() => {
    document.title = runtimeConfig().title
    // Asked here rather than when the store loads: the shell is the first thing
    // that exists, and a dialog decided at import time would be decided before
    // the page had rendered.
    askIfNeeded()
  }, [askIfNeeded])

  useEffect(() => {
    void refreshAccess()
  }, [token, refreshAccess])

  // Unauthenticated, so it does not wait for a token and does not refetch when one
  // arrives.
  useEffect(() => {
    void loadSchema()
  }, [loadSchema])

  return (
    <div className="mx-auto flex min-h-screen max-w-5xl flex-col px-4 py-4">
      <Header />
      <main className="grow py-4">
        <Suspense fallback={<Spinner label="Loading..." />}>
          <Outlet />
        </Suspense>
      </main>
      <Footer />
      <TokenDialog />
      <Toasts />
    </div>
  )
}
