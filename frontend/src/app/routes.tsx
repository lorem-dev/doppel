import { lazy } from 'react'
import { Route, Routes } from 'react-router'

import { App } from './App'

// Every page is its own chunk. That is the requirement -- chunks and lazy
// loading -- and it is also what keeps the entry payload to the shell: someone
// who opens the proxy list never downloads the template editor or prism.
const ProxiesPage = lazy(() => import('../pages/ProxiesPage'))
const ProxyFormPage = lazy(() => import('../pages/ProxyFormPage'))
const StatusPage = lazy(() => import('../pages/StatusPage'))
const TemplatesPage = lazy(() => import('../pages/TemplatesPage'))

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
export function AppRoutes() {
  return (
    <Routes>
      <Route path="/" element={<App />}>
        <Route index element={<ProxiesPage />} />
        <Route path="proxies/new" element={<ProxyFormPage />} />
        <Route path="proxies/:name" element={<ProxyFormPage />} />
        <Route path="proxies/:name/templates" element={<TemplatesPage />} />
        <Route path="status" element={<StatusPage />} />
      </Route>
    </Routes>
  )
}
