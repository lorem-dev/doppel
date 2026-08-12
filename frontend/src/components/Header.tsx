import { NavLink } from 'react-router'

import { Button } from './Button'
import { ThemeToggle } from './ThemeToggle'
import { runtimeConfig } from '../services/runtimeConfig'
import { useAccess } from '../store/access'
import { useAuth } from '../store/auth'

const TABS = [
  { to: '/', label: 'Proxies' },
  { to: '/status', label: 'Status' },
]

/**
 * The title, the tabs, and the two controls that are always available.
 *
 * The sign-in control is absent entirely on a public deployment: there are no
 * tokens to present, so offering the dialog would be offering something that
 * does not exist.
 */
export function Header() {
  const { title, public: isPublic } = runtimeConfig()
  const token = useAuth((state) => state.token)
  const openDialog = useAuth((state) => state.openDialog)
  const signOut = useAuth((state) => state.signOut)
  const caller = useAccess((state) => state.report?.caller)

  return (
    <header className="flex flex-wrap items-center gap-4 border-b border-slate-200 pb-3 dark:border-slate-800">
      <h1 className="text-lg font-semibold text-slate-900 dark:text-slate-100">{title}</h1>

      <nav className="flex gap-3 text-sm">
        {TABS.map(({ to, label }) => (
          <NavLink
            key={to}
            to={to}
            end
            className={({ isActive }) =>
              isActive
                ? 'font-medium text-teal-700 dark:text-teal-300'
                : 'text-slate-600 hover:text-slate-900 dark:text-slate-300 dark:hover:text-white'
            }
          >
            {label}
          </NavLink>
        ))}
      </nav>

      <div className="ml-auto flex items-center gap-3">
        {caller?.kind === 'token' ? (
          <span className="text-xs text-slate-500 dark:text-slate-400">
            {caller.name} ({caller.group})
          </span>
        ) : null}
        <ThemeToggle />
        {isPublic ? null : token ? (
          <Button onClick={signOut}>Sign out</Button>
        ) : (
          <Button onClick={openDialog}>Sign in</Button>
        )}
      </div>
    </header>
  )
}
