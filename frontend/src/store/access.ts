// What the caller may do, as the server reports it.
//
// Consulted by every control that performs an action. The answer comes from
// `GET /api/v1/access`, never from anything the page works out for itself: the
// rules are per-proxy overrides on top of a global block on top of a `public`
// flag, and a second implementation of that would eventually disagree with the
// one enforcing it.

import { useCallback } from 'react'
import { create } from 'zustand'

import type { AccessReport, Action, ProxyAction } from '../types/api'
import { ApiError } from '../types/error'
import { fetchAccess } from '../services/access'

interface AccessState {
  report?: AccessReport
  error?: ApiError
  refresh: () => Promise<void>
}

/**
 * Whether `report` permits `action`, optionally against one proxy.
 *
 * `false` without a report. The alternative -- assume permitted, find out on
 * click -- is exactly the button-that-does-nothing `/api/v1/access` exists to
 * prevent, and a control disabled for the length of one request is the smaller
 * cost.
 *
 * A free function rather than a method on the store: the precedence rules are the
 * part worth getting right, and this is the only copy of them. `useMay` binds it
 * to the current report for a component.
 */
export function permits(
  report: AccessReport | undefined,
  action: Action,
  proxy?: string,
): boolean {
  if (!report) {
    return false
  }
  // A proxy's own `access` block wins over the global one for the four actions it
  // may override -- the same precedence the server applies.
  if (proxy && report.proxies && isProxyAction(action)) {
    const entry = report.proxies[proxy]
    if (entry) {
      return entry[action]
    }
  }
  return report.global[action]
}

/**
 * `permits`, bound to the current report, for a component.
 *
 * Not `useAccess((state) => state.may)`. That looks equivalent and is not: the
 * store's `may` is one stable function reference, so selecting it subscribes a
 * component to nothing -- the report arrives, the reference does not change, and
 * every control stays disabled for the life of the page. The Playwright suite
 * caught exactly that. Selecting the report is what makes the re-render happen.
 */
export function useMay(): (action: Action, proxy?: string) => boolean {
  const report = useAccess((state) => state.report)
  return useCallback(
    (action: Action, proxy?: string) => permits(report, action, proxy),
    [report],
  )
}

export const useAccess = create<AccessState>((set) => ({
  refresh: async () => {
    try {
      set({ report: await fetchAccess(), error: undefined })
    } catch (caught) {
      // The endpoint answers 200 for everybody, so a failure here is the server
      // being unreachable rather than the caller being refused. Keeping the last
      // report would leave controls enabled against a server that is not
      // answering, so it is dropped.
      set({ report: undefined, error: caught as ApiError })
    }
  },

}))

/** `list` and `create` are not about one proxy, so a proxy cannot override them. */
function isProxyAction(action: Action): action is ProxyAction {
  return action === 'read' || action === 'update' || action === 'delete' || action === 'upload'
}
