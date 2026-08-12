import { cloneElement, isValidElement, useId, type ReactNode } from 'react'

import { Info } from './Info'

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
 *
 * The label names the control by `for` rather than by wrapping it, and the (i) sits
 * beside the label rather than inside it. Both follow from one rule: what a screen
 * reader announces for the input has to be the field's name and nothing else. A link
 * inside the label became part of that name -- "Timeout (seconds) What Timeout
 * (seconds) does, in the documentation" -- which is the kind of thing only a test
 * notices.
 */
export function Field({
  label,
  hint,
  error,
  children,
  htmlFor,
  info,
}: {
  label: string
  hint?: string
  error?: string
  children: ReactNode
  /**
   * The field's path in a proxy document, which makes the label carry an (i).
   *
   * `timeout`, `loss.percentage`, `mocks[].request.url`: the same paths the schema
   * rules use, so a field states its path once and gets both its bounds and its
   * documentation from it.
   */
  info?: string
  /**
   * The id of the control, when this component cannot give it one.
   *
   * The code editor renders its own textarea, so only the editor can put an id on
   * it. Everything else gets one from here.
   */
  htmlFor?: string
}) {
  const generated = useId()
  const id = htmlFor ?? generated
  // The hint and the complaint are about the control, so they are attached to it
  // rather than merely printed under it: a screen reader then reads "Upstream URL,
  // must be absolute" instead of leaving the reason somewhere on the page. It is also
  // what lets a test ask a field for its message without walking the DOM.
  const messageId = `${id}-message`
  const message = error ?? hint

  // The child is one element -- an input, a select, or an editor's Suspense -- and it
  // needs the id the label points at. Cloned rather than demanded of every call site:
  // twenty fields would each be spelling out an id whose only reader is the label
  // above it. A caller that owns the id says so with `htmlFor` and is left alone.
  const control =
    htmlFor === undefined && isValidElement<{ id?: string; 'aria-describedby'?: string }>(children)
      ? cloneElement(children, {
          id: children.props.id ?? id,
          'aria-describedby': message ? messageId : undefined,
        })
      : children

  return (
    <div className="block">
      {/*
        The margin belongs to the row, not to the label: with it on the label, the row
        centred a 14px circle against a 24px box and the icon sat two pixels low.
      */}
      <div className="mb-1 flex items-center gap-1">
        <label className={labelTextClass} htmlFor={id}>
          {label}
        </label>
        {info ? <Info path={info} label={label} /> : null}
      </div>
      {control}
      {message ? (
        <span
          id={messageId}
          className={`mt-1 block min-h-4 text-xs ${
            error ? 'text-red-700 dark:text-red-300' : 'text-slate-500 dark:text-slate-400'
          }`}
          role={error ? 'alert' : undefined}
        >
          {message}
        </span>
      ) : null}
    </div>
  )
}

/**
 * What a control's name looks like above it.
 *
 * Exported because the code editor cannot use `Field`: the library renders its own
 * textarea, so its name has to be a `label` with a `for` rather than a `span`
 * wrapping the control. Same words, same weight, one definition.
 */
export const labelTextClass = 'block text-sm font-medium text-slate-700 dark:text-slate-200'

/** The same, with the gap to the control -- for a label that stands on its own row. */
export const labelClass = `mb-1 ${labelTextClass}`

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
