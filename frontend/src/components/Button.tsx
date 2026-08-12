import type { ButtonHTMLAttributes, ReactNode } from 'react'

/**
 * A button that says why it is disabled.
 *
 * `reason` becomes the tooltip and the accessible description. A control the
 * caller's token may not use is disabled rather than hidden -- a missing button
 * reads as a broken page, a disabled one explains itself -- and that only works
 * if the explanation travels with it.
 */
export function Button({
  children,
  variant = 'plain',
  reason,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  children: ReactNode
  variant?: 'primary' | 'plain' | 'danger'
  reason?: string
}) {
  const tone = {
    primary: 'bg-teal-600 text-white hover:bg-teal-500',
    plain: 'border border-slate-300 text-slate-700 hover:bg-slate-100 dark:border-slate-600 dark:text-slate-200 dark:hover:bg-slate-800',
    danger: 'border border-red-300 text-red-700 hover:bg-red-50 dark:border-red-800 dark:text-red-300 dark:hover:bg-red-950',
  }[variant]

  return (
    <button
      type={rest.type ?? 'button'}
      title={rest.disabled ? reason : undefined}
      className={`rounded px-3 py-1 text-sm disabled:cursor-not-allowed disabled:opacity-50 ${tone}`}
      {...rest}
    >
      {children}
    </button>
  )
}
