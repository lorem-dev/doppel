import { REPOSITORY, docsRoot } from '../services/docs'
import { runtimeConfig } from '../services/runtimeConfig'

/**
 * The copyright, the version of the binary serving this page, and where to read
 * more.
 *
 * The version comes from the injected configuration rather than from
 * `package.json`: what matters is which Doppel answered, and a number baked into
 * the bundle would keep claiming the version the assets were built at.
 *
 * The year is the build's, and arrives the same way. A `new Date()` here would
 * report the reader's clock, which is neither when this was published nor the same
 * answer for two people looking at one binary.
 *
 * The documentation link carries the running version, like every other one out of
 * this page: the site is published per version, and an unversioned link would show
 * whoever followed it the rules of whatever has been released since.
 *
 * The copyright is text and stays text. It is a statement rather than a
 * destination, and a line that looks clickable and does nothing is worse than a
 * line that plainly does not. The two links beside it are the things there is
 * somewhere to go to, and they open in a new tab: both leave the dashboard, and
 * neither is worth losing a half-filled form for.
 */
export function Footer() {
  const { version, copyrightYear, titleIsDefault } = runtimeConfig()
  // A deployment that named itself is somebody else's tool, built with this one.
  // The line says so, the link says whose documentation it goes to, and the
  // repository link goes: on `billing-api (staging)`, "Repository" reads as a link
  // to the billing API's own, which it is not.
  const links = titleIsDefault
    ? [
        { href: REPOSITORY, label: 'Repository' },
        { href: docsRoot(), label: 'Documentation' },
      ]
    : [{ href: docsRoot(), label: 'Doppel Documentation' }]
  return (
    <footer className="mt-8 flex flex-wrap items-center justify-between gap-x-4 gap-y-1 border-t border-slate-200 py-4 text-xs text-slate-500 dark:border-slate-800 dark:text-slate-400">
      <p>
        {titleIsDefault ? null : `Built with Doppel ${version} `}
        (c) {copyrightYear} Lorem Dev
        {titleIsDefault ? ` \u00b7 Doppel ${version}` : null}
      </p>
      <nav className="flex items-center gap-3">
        {links.map(({ href, label }) => (
          <a
            key={href}
            href={href}
            target="_blank"
            rel="noreferrer"
            className="hover:text-slate-900 hover:underline dark:hover:text-slate-100"
          >
            {label}
            <span aria-hidden="true" className="ml-0.5">
              &#8599;
            </span>
          </a>
        ))}
      </nav>
    </footer>
  )
}
