import { lazy } from 'react'
import { Navigate, Route, Routes, useParams } from 'react-router'

import { App } from './App'

// Every page is its own chunk. That is the requirement -- chunks and lazy
// loading -- and it is also what keeps the entry payload to the shell: someone
// who opens the proxy list never downloads the template editor or prism.
const ProxiesPage = lazy(() => import('../pages/ProxiesPage'))
const ProxyFormPage = lazy(() => import('../pages/ProxyFormPage'))
const StatusPage = lazy(() => import('../pages/StatusPage'))

/**
 * The route table, in react-router's component form.
 *
 * Deliberately not `createBrowserRouter`. The data router brings loaders,
 * actions, fetchers and their state machine, and this app uses none of it -- the
 * pages fetch in an effect and the stores hold what outlives a screen. Measured
 * on the first build: the data router put the entry chunk at 90 KB gzipped and
 * this leaves it near 76, on a budget written for the smallest static payload
 * that will do the job.
 */
/** Where a bookmarked templates URL goes now. */
function ToProxy() {
  const { name = '' } = useParams()
  return <Navigate to={`/proxies/${encodeURIComponent(name)}`} replace />
}

export function AppRoutes() {
  return (
    <Routes>
      <Route path="/" element={<App />}>
        <Route index element={<ProxiesPage />} />
        <Route path="proxies/new" element={<ProxyFormPage />} />
        <Route path="proxies/:name" element={<ProxyFormPage />} />
        {/* Templates used to be a page of their own. They are a section of the
            proxy's form now -- a mock names a file and the file has to be written,
            and doing that on two screens meant carrying a name across a navigation.
            The old address still resolves, because it was linkable. */}
        <Route path="proxies/:name/templates" element={<ToProxy />} />
        <Route path="status" element={<StatusPage />} />
      </Route>
    </Routes>
  )
}
