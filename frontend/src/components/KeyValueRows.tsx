import { Button } from './Button'
import { inputClass } from './Field'

/**
 * A string-to-string map as rows.
 *
 * Used for injected headers and for every selector map a mock declares. Keyed by
 * index rather than by the key itself: keying by the key would remount the input
 * on every keystroke that changes it, and the field would lose focus after one
 * character.
 */
export function KeyValueRows({
  label,
  keyLabel,
  valueLabel,
  rows,
  onChange,
  disabled,
}: {
  label: string
  keyLabel: string
  valueLabel: string
  rows: Array<[string, string]>
  onChange: (rows: Array<[string, string]>) => void
  disabled?: boolean
}) {
  const set = (index: number, pair: [string, string]) => {
    const next = [...rows]
    next[index] = pair
    onChange(next)
  }

  return (
    <fieldset className="mt-2">
      <legend className="text-sm font-medium text-slate-700 dark:text-slate-200">{label}</legend>
      <div className="flex flex-col gap-1">
        {rows.map(([key, value], index) => (
          <div key={index} className="flex items-center gap-2">
            <input
              className={inputClass}
              aria-label={`${label} ${keyLabel} ${index + 1}`}
              placeholder={keyLabel}
              value={key}
              disabled={disabled}
              onChange={(event) => set(index, [event.target.value, value])}
            />
            <input
              className={inputClass}
              aria-label={`${label} ${valueLabel} ${index + 1}`}
              placeholder={valueLabel}
              value={value}
              disabled={disabled}
              onChange={(event) => set(index, [key, event.target.value])}
            />
            <Button
              variant="danger"
              disabled={disabled}
              onClick={() => onChange(rows.filter((_, at) => at !== index))}
            >
              Remove
            </Button>
          </div>
        ))}
      </div>
      <Button
        disabled={disabled}
        onClick={() => onChange([...rows, ['', '']])}
        aria-label={`Add ${label}`}
      >
        Add
      </Button>
    </fieldset>
  )
}

/** A map as rows, for editing. */
export function toRows(map: Record<string, string> | undefined): Array<[string, string]> {
  return Object.entries(map ?? {})
}

/**
 * Rows back to a map, dropping incomplete ones.
 *
 * An empty key cannot be sent: the server would refuse the document, and the row
 * is what an operator leaves behind after clicking Add and changing their mind.
 */
export function fromRows(rows: Array<[string, string]>): Record<string, string> | undefined {
  const map: Record<string, string> = {}
  for (const [key, value] of rows) {
    if (key.trim()) {
      map[key.trim()] = value
    }
  }
  return Object.keys(map).length ? map : undefined
}
