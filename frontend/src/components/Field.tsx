import type { ReactNode } from 'react'

/**
 * A labelled control with room for the server's complaint about it.
 *
 * `error` comes from the server, not from a second copy of the rules here: the
 * configuration types are the arbiter of what is legal, and a client-side
 * duplicate would be a second, laxer standard nobody wrote down.
 */
export function Field({
  label,
  hint,
  error,
  children,
}: {
  label: string
  hint?: string
  error?: string
  children: ReactNode
}) {
  return (
    <label className="block">
      <span className="block text-sm font-medium text-slate-700 dark:text-slate-200">{label}</span>
      {children}
      {hint && !error ? (
        <span className="mt-1 block text-xs text-slate-500 dark:text-slate-400">{hint}</span>
      ) : null}
      {error ? (
        <span className="mt-1 block text-xs text-red-700 dark:text-red-300" role="alert">
          {error}
        </span>
      ) : null}
    </label>
  )
}

/** The one input style, so every form field looks like the others. */
export const inputClass =
  'mt-1 w-full rounded border border-slate-300 bg-white px-2 py-1 text-sm text-slate-900 ' +
  'focus:border-teal-500 focus:outline-none disabled:opacity-50 ' +
  'dark:border-slate-600 dark:bg-slate-800 dark:text-slate-100'
