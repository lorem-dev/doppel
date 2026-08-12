/** Shown while the first load is in flight, never for a refetch. */
export function Spinner({ label }: { label: string }) {
  return (
    <p className="text-sm text-slate-500 dark:text-slate-400" role="status">
      {label}
    </p>
  )
}
