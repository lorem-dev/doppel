import { Button } from './Button'

/**
 * A yes-or-no question, for the one action that cannot be undone.
 *
 * Delete is the only caller. Nothing else here destroys anything an operator
 * could not put back by editing a field again.
 */
export function ConfirmDialog({
  question,
  detail,
  confirmLabel,
  onConfirm,
  onCancel,
}: {
  question: string
  detail?: string
  confirmLabel: string
  onConfirm: () => void
  onCancel: () => void
}) {
  return (
    <div className="fixed inset-0 z-10 flex items-center justify-center bg-slate-900/40 p-4">
      <div
        role="dialog"
        aria-label={question}
        className="w-full max-w-sm rounded border border-slate-200 bg-white p-4 shadow-lg dark:border-slate-700 dark:bg-slate-900"
      >
        <p className="text-sm font-medium text-slate-900 dark:text-slate-100">{question}</p>
        {detail ? (
          <p className="mt-1 text-sm text-slate-600 dark:text-slate-300">{detail}</p>
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
