import { useCallback, useState } from 'react'
import { Link, useNavigate } from 'react-router'

import { Banner } from '../components/Banner'
import { Button } from '../components/Button'
import { ConfirmDialog } from '../components/ConfirmDialog'
import { Spinner } from '../components/Spinner'
import { usePolling } from '../app/usePolling'
import type { ProxyConfig, ProxyView } from '../types/proxy'
import { ApiError } from '../types/error'
import { useMay } from '../store/access'
import { useAuth } from '../store/auth'
import { useProxies } from '../store/proxies'
import { useToasts } from '../store/toast'

/**
 * The fault settings, one short label each.
 *
 * Separate rather than joined into a sentence: as one string it wrapped across two
 * lines in a narrow column and pushed every other cell in the row out of line.
 */
function faults(proxy: ProxyConfig): string[] {
  const parts: string[] = []
  if (proxy.replace !== undefined) {
    parts.push(`replace ${percent(proxy.replace)}`)
  }
  if (proxy.loss) {
    parts.push(`loss ${percent(proxy.loss.percentage)} -> ${proxy.loss.status}`)
  }
  if (proxy.latency) {
    parts.push(
      `latency ${percent(proxy.latency.percentage)} ${proxy.latency.min}-${proxy.latency.max}s`,
    )
  }
  return parts
}

/** A ratio as a percentage, because the column is for reading, not editing. */
function percent(ratio: number): string {
  return `${Math.round(ratio * 100)}%`
}

function resolveOf(proxy: ProxyConfig): string {
  if (proxy.resolve?.type !== 'header') {
    return 'default'
  }
  return proxy.resolve.header ? `header:${proxy.resolve.header}` : 'header'
}

export default function ProxiesPage() {
  const items = useProxies((state) => state.items)
  const loading = useProxies((state) => state.loading)
  const error = useProxies((state) => state.error)
  const load = useProxies((state) => state.load)
  const remove = useProxies((state) => state.remove)
  const may = useMay()
  const openDialog = useAuth((state) => state.openDialog)
  const token = useAuth((state) => state.token)
  const push = useToasts((state) => state.push)
  const navigate = useNavigate()
  const [pending, setPending] = useState<ProxyView>()

  // Once a minute, immediately on arrival, and again the moment the token changes:
  // signing in has to clear a refusal that is already on screen, and signing out has
  // to stop showing what the token could see. `load` is stable, so this does not
  // resubscribe on every render.
  usePolling(
    useCallback(() => void load(), [load]),
    token,
  )

  const onDelete = (view: ProxyView) => {
    remove(view.proxy.name, view.revision)
      .then(() => push('done', `Deleted ${view.proxy.name}`))
      .catch((caught: ApiError) =>
        push(
          'failed',
          caught.isStale
            ? `${view.proxy.name} changed since this page loaded; it was not deleted`
            : caught.message,
        ),
      )
      .finally(() => setPending(undefined))
  }

  return (
    <section className="flex flex-col gap-4">
      <div className="flex items-center gap-3">
        <h2 className="text-base font-semibold text-slate-900 dark:text-slate-100">Proxies</h2>
        <Button
          variant="primary"
          onClick={() => void navigate('/proxies/new')}
          disabled={!may('create')}
          reason="This token may not create proxies"
        >
          Add a proxy
        </Button>
      </div>

      {error ? (
        <Banner
          kind="error"
          action={
            error.isAuth ? (
              <Button onClick={openDialog}>Enter token</Button>
            ) : (
              <Button onClick={() => void load()}>Retry</Button>
            )
          }
        >
          {error.message}
        </Banner>
      ) : null}

      {loading ? <Spinner label="Reading the proxy set..." /> : null}

      {!loading && items.length === 0 && !error ? (
        <Banner kind="note">
          No proxies are configured. Doppel answers 503 with NO_PROXIES_CONFIGURED until one is.
        </Banner>
      ) : null}

      {items.length ? (
        <div className="overflow-x-auto rounded-md border border-slate-200 dark:border-slate-800">
          <table className="w-full border-collapse text-left text-sm">
            <thead className="bg-slate-50 text-xs uppercase tracking-wide text-slate-500 dark:bg-slate-900 dark:text-slate-400">
              <tr>
                <th scope="col" className="px-3 py-2 font-medium">
                  Name
                </th>
                <th scope="col" className="px-3 py-2 font-medium">
                  Upstream
                </th>
                <th scope="col" className="px-3 py-2 font-medium">
                  Resolve
                </th>
                <th scope="col" className="px-3 py-2 font-medium">
                  Faults
                </th>
                {/* Right-aligned because it is a number, and numbers line up on
                    their last digit. */}
                <th scope="col" className="px-3 py-2 text-right font-medium">
                  Mocks
                </th>
                <th scope="col" className="px-3 py-2" />
              </tr>
            </thead>
            <tbody>
              {items.map(({ revision, proxy }) => (
                <tr
                  key={proxy.name}
                  className="border-t border-slate-200 align-middle hover:bg-slate-50 dark:border-slate-800 dark:hover:bg-slate-900/60"
                >
                  <td className="px-3 py-2 font-medium whitespace-nowrap text-slate-900 dark:text-slate-100">
                    <Link
                      className="hover:underline"
                      to={`/proxies/${encodeURIComponent(proxy.name)}`}
                    >
                      {proxy.name}
                    </Link>
                  </td>
                  {/* Truncated with the whole value in the title: a long upstream
                      used to wrap across two lines and push every other cell in the
                      row out of alignment. */}
                  <td className="max-w-xs truncate px-3 py-2 font-mono text-xs text-slate-700 dark:text-slate-300">
                    <span title={proxy.url}>{proxy.url}</span>
                  </td>
                  <td className="px-3 py-2 whitespace-nowrap text-slate-700 dark:text-slate-300">
                    {resolveOf(proxy)}
                  </td>
                  <td className="px-3 py-2 text-slate-700 dark:text-slate-300">
                    <div className="flex flex-wrap gap-1">
                      {faults(proxy).map((fault) => (
                        <span
                          key={fault}
                          className="rounded bg-slate-100 px-1.5 py-0.5 font-mono text-xs whitespace-nowrap text-slate-700 dark:bg-slate-800 dark:text-slate-300"
                        >
                          {fault}
                        </span>
                      ))}
                      {faults(proxy).length === 0 ? (
                        <span className="text-slate-400 dark:text-slate-500">none</span>
                      ) : null}
                    </div>
                  </td>
                  <td className="px-3 py-2 text-right tabular-nums text-slate-700 dark:text-slate-300">
                    {proxy.mocks?.length ?? 0}
                  </td>
                  <td className="px-3 py-2">
                    <div className="flex justify-end gap-2">
                      <Link
                        className="inline-flex h-9 shrink-0 items-center rounded-md border border-slate-300 px-3 text-sm font-medium text-slate-700 hover:bg-slate-100 dark:border-slate-600 dark:text-slate-200 dark:hover:bg-slate-800"
                        to={`/proxies/${encodeURIComponent(proxy.name)}/templates`}
                      >
                        Templates
                      </Link>
                      <Button
                        variant="danger"
                        onClick={() => setPending({ revision, proxy })}
                        disabled={!may('delete', proxy.name)}
                        reason="This token may not delete this proxy"
                      >
                        Delete
                      </Button>
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : null}

      {pending ? (
        <ConfirmDialog
          question={`Delete ${pending.proxy.name}?`}
          detail="Its template files are left on disk. This cannot be undone."
          confirmLabel="Delete"
          onConfirm={() => onDelete(pending)}
          onCancel={() => setPending(undefined)}
        />
      ) : null}
    </section>
  )
}
