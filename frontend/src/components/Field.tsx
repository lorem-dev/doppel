import type { ReactNode } from 'react'

/**
 * A labelled control with room for the server's complaint about it.
 *
 * `error` comes from the server, not from a second copy of the rules here: the
 * configuration types are the arbiter of what is legal, and a client-side
 * duplicate would be a second, laxer standard nobody wrote down.
 *
 * The hint and the error share one line of reserved space. Without that the row
 * grows the moment a save is refused and everything below it jumps -- which is
 * exactly when the operator is trying to read something.
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
      <span className={labelClass}>{label}</span>
      {children}
      {hint || error ? (
        <span
          className={`mt-1 block min-h-4 text-xs ${
            error ? 'text-red-700 dark:text-red-300' : 'text-slate-500 dark:text-slate-400'
          }`}
          role={error ? 'alert' : undefined}
        >
          {error ?? hint}
        </span>
      ) : null}
    </label>
  )
}

/**
 * What a control's name looks like above it.
 *
 * Exported because the code editor cannot use `Field`: the library renders its own
 * textarea, so its name has to be a `label` with a `for` rather than a `span`
 * wrapping the control. Same words, same weight, one definition.
 */
export const labelClass = 'mb-1 block text-sm font-medium text-slate-700 dark:text-slate-200'

/**
 * One height and one padding for every control on the page.
 *
 * `h-9` is not decoration. An input, a select and a button sat side by side at
 * three different heights, so a row of them stepped up and down -- and a form is
 * mostly rows of them. Anything that shares a line with these has to share this
 * height, which is why the button uses it too.
 */
export const controlClass =
  'block h-9 w-full rounded-md border border-slate-300 bg-white px-2.5 text-sm ' +
  'text-slate-900 placeholder:text-slate-400 focus:border-teal-500 focus:ring-1 ' +
  'focus:ring-teal-500/40 focus:outline-none disabled:cursor-not-allowed ' +
  'disabled:opacity-60 dark:border-slate-600 dark:bg-slate-900 dark:text-slate-100 ' +
  'dark:placeholder:text-slate-500'

/**
 * A select, which needs more room on the end than an input does.
 *
 * The browser draws the arrow inside the padding, so without this it sits against
 * the border and the value it belongs to reads as if it had been cut off.
 */
export const selectClass = `${controlClass} appearance-none bg-no-repeat pr-8`

/**
 * Every select: the control, the room for an arrow, and the arrow.
 *
 * `appearance-none` removes the native one, which is what put it against the border
 * and made a select a different height from an input. `select-chevron` is in
 * `index.css` -- see the comment there for why it is not a Tailwind arbitrary
 * value.
 */
export const selectFullClass = `${selectClass} select-chevron`
