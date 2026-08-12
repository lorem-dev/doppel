import { NavLink } from 'react-router'

import { Button } from './Button'
import { ThemeToggle } from './ThemeToggle'
import { Title } from './Title'
import { Wordmark } from './Wordmark'
import { runtimeConfig } from '../services/runtimeConfig'
import { useAccess } from '../store/access'
import { useAuth } from '../store/auth'

const TABS = [
  { to: '/', label: 'Proxies' },
  { to: '/status', label: 'Status' },
]

/**
 * Swagger UI, which the server serves rather than the page.
 *
 * A plain anchor, not a `NavLink`: it lives under `/api/`, which react-router must
 * not try to resolve as one of its own routes. In a new tab, because it is a
 * different application and going there should not throw away a half-filled form.
 */
const SWAGGER = '/swagger-ui/'

/**
 * The title, the tabs, and the two controls that are always available.
 *
 * The sign-in control is absent entirely on a public deployment: there are no
 * tokens to present, so offering the dialog would be offering something that
 * does not exist.
 */
export function Header() {
  const { title, titleIsDefault, public: isPublic } = runtimeConfig()
  const token = useAuth((state) => state.token)
  const openDialog = useAuth((state) => state.openDialog)
  const signOut = useAuth((state) => state.signOut)
  const caller = useAccess((state) => state.report?.caller)

  return (
    <header className="flex flex-wrap items-center gap-4 border-b border-slate-200 pb-3 dark:border-slate-800">
      {/*
        The heading is a link home, which is what a title in this position is
        everywhere else. It costs nothing -- the list is one navigation, not a reload
        -- and it is the affordance people try first when a form has taken them
        somewhere they did not mean to be.
      */}
      <h1 className="text-lg font-semibold text-slate-900 dark:text-slate-100">
        <NavLink to="/" end className="rounded-sm hover:opacity-80">
          {titleIsDefault ? <Wordmark /> : <Title title={title} />}
        </NavLink>
      </h1>

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
        <a
          href={SWAGGER}
          target="_blank"
          rel="noreferrer"
          className="text-slate-600 hover:text-slate-900 dark:text-slate-300 dark:hover:text-white"
        >
          API
          <span aria-hidden="true" className="ml-0.5 text-xs">
            &#8599;
          </span>
        </a>
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
