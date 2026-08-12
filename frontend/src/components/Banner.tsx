import type { ReactNode } from 'react'

/**
 * A message the operator has to read before the page makes sense.
 *
 * `action` is for the affordance that fixes it -- "Enter token" beside a 403 --
 * because a message that names a problem without offering the fix leaves the
 * reader to work out which of the header's controls they were supposed to use.
 */
export function Banner({
  kind,
  children,
  action,
}: {
  kind: 'error' | 'warning' | 'note'
  children: ReactNode
  action?: ReactNode
}) {
  // Three tones, because the three say different things: something is wrong, this
  // will not work yet, and here is a fact. A warning shown in the error's colour
  // reads as a failure that already happened.
  const tone = {
    error:
      'border-red-300 bg-red-50 text-red-900 dark:border-red-800 dark:bg-red-950 dark:text-red-100',
    warning:
      'border-amber-300 bg-amber-50 text-amber-900 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-100',
    note: 'border-slate-300 bg-slate-50 text-slate-800 dark:border-slate-700 dark:bg-slate-900 dark:text-slate-100',
  }[kind]

  return (
    <div className={`flex items-start gap-3 rounded border px-3 py-2 text-sm ${tone}`} role="alert">
      <div className="grow whitespace-pre-wrap">{children}</div>
      {action}
    </div>
  )
}
