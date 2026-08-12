import { runtimeConfig } from '../services/runtimeConfig'

/**
 * The copyright and the version of the binary serving this page.
 *
 * The version comes from the injected configuration rather than from
 * `package.json`: what matters is which Doppel answered, and a number baked into
 * the bundle would keep claiming the version the assets were built at.
 */
export function Footer() {
  const { version } = runtimeConfig()
  return (
    <footer className="mt-8 border-t border-slate-200 py-4 text-center text-xs text-slate-500 dark:border-slate-800 dark:text-slate-400">
      (c) 2026 Lorem Dev &middot; Doppel {version}
    </footer>
  )
}
