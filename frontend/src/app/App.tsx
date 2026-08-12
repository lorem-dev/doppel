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
import { startTheme } from '../store/theme'

/**
 * The shell every page is drawn inside.
 *
 * It owns two things no page should own: the theme's lifetime, and refetching the
 * caller's rights whenever the token changes. Rights are per-token, so a page
 * that fetched them for itself would fetch them again on every navigation and
 * still miss a sign-out that happened elsewhere.
 */
export function App() {
  const token = useAuth((state) => state.token)
  const askIfNeeded = useAuth((state) => state.askIfNeeded)
  const refreshAccess = useAccess((state) => state.refresh)

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
