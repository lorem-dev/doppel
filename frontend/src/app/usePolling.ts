import { useEffect } from 'react'

import { runtimeConfig } from '../services/runtimeConfig'

/**
 * Run `tick` now, then every `refreshMs`, but not while the tab is hidden.
 *
 * The interval comes from the injected configuration, so "once a minute" is one
 * value decided by the server rather than a number written in two places.
 *
 * Pausing on hidden is not an optimisation for the browser's sake: a dashboard
 * left open in a background tab overnight would otherwise make about a thousand
 * requests nobody reads, and each one carries the admin token.
 *
 * `key` is never read. It is the thing whose change means "and fetch again now":
 * the token, for the caller below. Without it, signing in left the refusal that
 * had been shown to an anonymous caller on screen until the next tick -- up to a
 * minute of a page saying a token is needed to someone who had just given one.
 */
export function usePolling(tick: () => void, key?: unknown): void {
  useEffect(() => {
    const { refreshMs } = runtimeConfig()

    // Fired immediately as well as on the interval, so opening a page does not
    // show an empty table for a minute.
    tick()

    let timer = window.setInterval(tick, refreshMs)

    const onVisibility = () => {
      window.clearInterval(timer)
      if (document.visibilityState === 'visible') {
        // Refetched on the way back, because what is on screen is now as stale as
        // the time the tab spent hidden.
        tick()
        timer = window.setInterval(tick, refreshMs)
      }
    }

    document.addEventListener('visibilitychange', onVisibility)
    return () => {
      window.clearInterval(timer)
      document.removeEventListener('visibilitychange', onVisibility)
    }
  }, [tick, key])
}
