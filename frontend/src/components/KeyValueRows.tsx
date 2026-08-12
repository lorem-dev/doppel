import { useState } from 'react'

import { Button } from './Button'
import { controlClass } from './Field'

/**
 * A string-to-string map as rows.
 *
 * Used for injected headers and for every selector map a mock declares. Keyed by
 * index rather than by the key itself: keying by the key would remount the input
 * on every keystroke that changes it, and the field would lose focus after one
 * character.
 *
 * The rows are local state, and that is the whole design rather than a
 * convenience. A row being typed is not yet a map entry -- it has no key -- so
 * deriving the rows from the map on every render made "Add" do nothing at all: the
 * new row had an empty key, the map could not hold it, and the next render had
 * nothing to show. The map is what leaves this component; the rows are what lives
 * in it.
 */
export function KeyValueRows({
  label,
  keyLabel,
  valueLabel,
  value,
  onChange,
  disabled,
}: {
  label: string
  keyLabel: string
  valueLabel: string
  /** The map as it stands. Incomplete rows are this component's business. */
  value: Record<string, string> | undefined
  onChange: (next: Record<string, string> | undefined) => void
  disabled?: boolean
}) {
  const [rows, setRows] = useState<Array<[string, string]>>(() => toRows(value))

  // Reseeded when the map arrives holding something the rows do not describe -- a
  // proxy being loaded, or a form being reset. Compared against what the rows
  // produce rather than against the previous prop, because this component's own
  // edit arrives back as a prop on the very next render and must not disturb a row
  // being typed.
  //
  // Adjusted during render rather than in an effect: React re-renders immediately
  // without committing the stale pass, so there is no flash of the old rows, and an
  // effect that set state here would run a render behind and fight the input.
  if (!sameMap(fromRows(rows), value)) {
    setRows(toRows(value))
  }

  const publish = (next: Array<[string, string]>) => {
    setRows(next)
    onChange(fromRows(next))
  }

  const set = (index: number, pair: [string, string]) => {
    const next = [...rows]
    next[index] = pair
    publish(next)
  }

  return (
    <fieldset>
      <legend className="mb-1.5 text-sm font-medium text-slate-700 dark:text-slate-200">
        {label}
      </legend>
      <div className="flex flex-col gap-2">
        {rows.map(([key, value], index) => (
          <div key={index} className="flex items-center gap-2">
            <input
              className={controlClass}
              aria-label={`${label} ${keyLabel} ${index + 1}`}
              placeholder={keyLabel}
              value={key}
              disabled={disabled}
              onChange={(event) => set(index, [event.target.value, value])}
            />
            <input
              className={controlClass}
              aria-label={`${label} ${valueLabel} ${index + 1}`}
              placeholder={valueLabel}
              value={value}
              disabled={disabled}
              onChange={(event) => set(index, [key, event.target.value])}
            />
            <Button
              variant="danger"
              disabled={disabled}
              onClick={() => publish(rows.filter((_, at) => at !== index))}
            >
              Remove
            </Button>
          </div>
        ))}
      </div>
      <div className={rows.length ? 'mt-2' : ''}>
        <Button
          disabled={disabled}
          onClick={() => publish([...rows, ['', '']])}
          aria-label={`Add ${label}`}
        >
          Add
        </Button>
      </div>
    </fieldset>
  )
}

/** Whether two of these maps hold the same pairs. */
function sameMap(
  left: Record<string, string> | undefined,
  right: Record<string, string> | undefined,
): boolean {
  const a = left ?? {}
  const b = right ?? {}
  const keys = Object.keys(a)
  return (
    keys.length === Object.keys(b).length && keys.every((key) => a[key] === b[key])
  )
}

/** A map as rows, for editing. */
function toRows(map: Record<string, string> | undefined): Array<[string, string]> {
  return Object.entries(map ?? {})
}

/**
 * Rows back to a map, dropping incomplete ones.
 *
 * An empty key cannot be sent: the server would refuse the document, and the row
 * is what an operator leaves behind after clicking Add and changing their mind.
 */
function fromRows(rows: Array<[string, string]>): Record<string, string> | undefined {
  const map: Record<string, string> = {}
  for (const [key, value] of rows) {
    if (key.trim()) {
      map[key.trim()] = value
    }
  }
  return Object.keys(map).length ? map : undefined
}
