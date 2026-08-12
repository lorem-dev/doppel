import { useState } from 'react'

import { Button } from './Button'
import { controlClass } from './Field'
import { useAuth } from '../store/auth'

/**
 * Ask for a token, and take no for an answer.
 *
 * "Continue without a token" is not politeness: `access.list` and `access.read`
 * can be `public`, and a proxy's own block can open one proxy to a group, so a
 * caller with no token may have plenty to look at. A dialog that could not be
 * dismissed would hide a working page behind a demand for a secret the operator
 * may not have and may not need.
 */
export function TokenDialog() {
  const open = useAuth((state) => state.dialogOpen)
  const signIn = useAuth((state) => state.signIn)
  const decline = useAuth((state) => state.decline)
  const close = useAuth((state) => state.closeDialog)
  const hasToken = useAuth((state) => state.token !== undefined)
  const [value, setValue] = useState('')

  if (!open) {
    return null
  }

  return (
    <div className="fixed inset-0 z-10 flex items-center justify-center bg-slate-900/40 p-4">
      <form
        className="w-full max-w-md rounded border border-slate-200 bg-white p-4 shadow-lg dark:border-slate-700 dark:bg-slate-900"
        aria-label="Admin token"
        onSubmit={(event) => {
          event.preventDefault()
          if (value.trim()) {
            signIn(value.trim())
            setValue('')
          }
        }}
      >
        <h2 className="text-base font-semibold text-slate-900 dark:text-slate-100">
          Admin token
        </h2>
        <p className="mt-1 text-sm text-slate-600 dark:text-slate-300">
          One of <code>admin.tokens</code>. Kept in this browser for an hour; the token itself
          does not expire.
        </p>
        <input
          // `mt-3`, matching the gap above the buttons below: the field sat
          // against the sentence explaining it, which read as one block.
          className={`mt-3 ${controlClass}`}
          type="password"
          autoComplete="off"
          aria-label="Token"
          value={value}
          onChange={(event) => setValue(event.target.value)}
        />
        <div className="mt-3 flex justify-end gap-2">
          <Button
            onClick={() => {
              if (hasToken) {
                close()
              } else {
                decline()
              }
            }}
          >
            {hasToken ? 'Cancel' : 'Continue without a token'}
          </Button>
          <Button type="submit" variant="primary" disabled={!value.trim()}>
            Use this token
          </Button>
        </div>
      </form>
    </div>
  )
}
