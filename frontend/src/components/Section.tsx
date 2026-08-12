import type { ReactNode } from 'react'

/**
 * A part of the form that starts folded.
 *
 * A proxy has thirteen fields and any number of mocks, and most edits touch one of
 * them. Laid out flat, the form was a page and a half of controls to scroll past to
 * reach the one being changed -- so every group folds, and starts folded.
 *
 * Native `<details>`: it is keyboard-operable, it is what a screen reader already
 * understands, and it needs no state, no effect and no library. `summary` carries a
 * count or a short description of what is inside, so a folded section still says
 * whether there is anything in it -- a row of identical closed headings would make
 * the operator open all four to find out.
 */
export function Section({
  title,
  summary,
  children,
  open,
}: {
  title: string
  /** What is inside, in a few words. Shown while folded and while open. */
  summary?: string
  children: ReactNode
  /** Folded unless this says otherwise. */
  open?: boolean
}) {
  return (
    <details
      open={open}
      className="group rounded-md border border-slate-200 bg-white/40 dark:border-slate-800 dark:bg-slate-900/40"
    >
      <summary className="flex cursor-pointer items-center gap-2 rounded-md px-3 py-2 text-sm font-semibold text-slate-900 marker:content-none hover:bg-slate-50 dark:text-slate-100 dark:hover:bg-slate-800/60">
        {/* Rotated rather than swapped: one glyph, and the turn is the animation. */}
        <span
          aria-hidden="true"
          className="text-slate-400 transition-transform group-open:rotate-90"
        >
          &#9656;
        </span>
        <span>{title}</span>
        {summary ? (
          <span className="font-normal text-slate-500 dark:text-slate-400">{summary}</span>
        ) : null}
      </summary>
      <div className="flex flex-col gap-3 border-t border-slate-200 px-3 py-3 dark:border-slate-800">
        {children}
      </div>
    </details>
  )
}
