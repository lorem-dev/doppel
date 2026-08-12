import type { ReactNode } from 'react'

import { Button } from './Button'

/**
 * A yes-or-no question, for an action that cannot be undone.
 *
 * Two callers: deleting a proxy, and saving a document that removes something. Both
 * are losses an operator cannot get back by editing a field again -- which is the
 * test for whether something belongs here.
 *
 * `detail` takes a node rather than a string because the second caller has a list to
 * show, and a list of removals reads better than a sentence naming five things.
 */
export function ConfirmDialog({
  question,
  detail,
  confirmLabel,
  onConfirm,
  onCancel,
}: {
  question: string
  detail?: ReactNode
  confirmLabel: string
  onConfirm: () => void
  onCancel: () => void
}) {
  return (
    <div className="fixed inset-0 z-10 flex items-center justify-center bg-slate-900/40 p-4">
      <div
        role="dialog"
        aria-label={question}
        className="w-full max-w-md rounded border border-slate-200 bg-white p-4 shadow-lg dark:border-slate-700 dark:bg-slate-900"
      >
        <p className="text-sm font-medium text-slate-900 dark:text-slate-100">{question}</p>
        {detail ? (
          <div className="mt-1 text-sm text-slate-600 dark:text-slate-300">{detail}</div>
        ) : null}
        <div className="mt-3 flex justify-end gap-2">
          <Button onClick={onCancel}>Cancel</Button>
          <Button variant="danger" onClick={onConfirm}>
            {confirmLabel}
          </Button>
        </div>
      </div>
    </div>
  )
}
