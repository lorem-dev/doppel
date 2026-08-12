import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { BrowserRouter } from 'react-router'

import { AppRoutes } from './app/routes'
import './index.css'

const root = document.getElementById('root')
if (!root) {
  throw new Error('index.html is missing #root')
}

// No basename: the dashboard is served from the admin listener's root, and
// `/static/` is the asset prefix rather than a route.
createRoot(root).render(
  <StrictMode>
    <BrowserRouter>
      <AppRoutes />
    </BrowserRouter>
  </StrictMode>,
)
