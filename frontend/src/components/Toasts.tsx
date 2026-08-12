import { useToasts } from '../store/toast'

/** What the last write did, in the corner, dismissible. */
export function Toasts() {
  const items = useToasts((state) => state.items)
  const dismiss = useToasts((state) => state.dismiss)

  if (items.length === 0) {
    return null
  }

  return (
    <div className="fixed bottom-4 right-4 z-20 flex w-80 flex-col gap-2">
      {items.map((toast) => (
        <button
          key={toast.id}
          type="button"
          onClick={() => dismiss(toast.id)}
          className={`rounded border px-3 py-2 text-left text-sm shadow ${
            toast.kind === 'failed'
              ? 'border-red-300 bg-red-50 text-red-900 dark:border-red-800 dark:bg-red-950 dark:text-red-100'
              : 'border-teal-300 bg-teal-50 text-teal-900 dark:border-teal-800 dark:bg-teal-950 dark:text-teal-100'
          }`}
        >
          {toast.text}
        </button>
      ))}
    </div>
  )
}
