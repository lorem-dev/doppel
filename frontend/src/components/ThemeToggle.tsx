import { useTheme } from '../store/theme'
import type { ThemeChoice } from '../store/theme'

const CHOICES: Array<{ value: ThemeChoice; label: string }> = [
  { value: 'light', label: 'Light' },
  { value: 'dark', label: 'Dark' },
  { value: 'system', label: 'System' },
]

/**
 * Three states, not two.
 *
 * A two-state switch cannot express "whatever the desktop says", which is the
 * default and the one most operators never change.
 */
export function ThemeToggle() {
  const choice = useTheme((state) => state.choice)
  const set = useTheme((state) => state.set)

  return (
    <label className="flex items-center gap-2 text-sm">
      <span className="sr-only">Theme</span>
      <select
        aria-label="Theme"
        value={choice}
        onChange={(event) => set(event.target.value as ThemeChoice)}
        className="rounded border border-slate-300 bg-white px-2 py-1 text-sm dark:border-slate-600 dark:bg-slate-800"
      >
        {CHOICES.map(({ value, label }) => (
          <option key={value} value={value}>
            {label}
          </option>
        ))}
      </select>
    </label>
  )
}
