import { Button } from './Button'
import type { ApiError } from '../types/error'
import { runtimeConfig } from '../services/runtimeConfig'
import { useAuth } from '../store/auth'

/**
 * What to offer beside a refusal: a way to fix it, and a way to ask again.
 *
 * "Read of `alpha` requires access `read`" is a message about the token, not about
 * the page, and every one of these screens reads once when it opens. So a refusal
 * used to sit there after the operator signed in, with nothing on it to press --
 * the fix was to reload the browser, which is a thing people do and should not have
 * to.
 *
 * Refresh rather than an automatic refetch when the token changes. The list can
 * refetch itself safely because it holds nothing; a form holds a half-typed
 * document, and replacing that from under someone because they signed in would
 * lose work to a keystroke they made elsewhere. So the page asks, and the operator
 * decides when.
 */
export function Retry({ error, onRetry }: { error: ApiError; onRetry: () => void }) {
  const openDialog = useAuth((state) => state.openDialog)
  // On a public deployment there are no tokens, so offering the dialog would offer
  // something that does not exist.
  const askable = error.isAuth && !runtimeConfig().public

  return (
    <div className="flex shrink-0 items-center gap-2">
      {askable ? <Button onClick={openDialog}>Enter token</Button> : null}
      <Button onClick={onRetry}>Refresh</Button>
    </div>
  )
}
