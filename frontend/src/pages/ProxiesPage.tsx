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

/** The fault settings, in one column instead of four. */
function faults(proxy: ProxyConfig): string {
  const parts: string[] = []
  if (proxy.replace !== undefined) {
    parts.push(`replace ${percent(proxy.replace)}`)
  }
  if (proxy.loss) {
    parts.push(`loss ${percent(proxy.loss.percentage)} -> ${proxy.loss.status}`)
  }
  if (proxy.latency) {
    parts.push(`latency ${percent(proxy.latency.percentage)} ${proxy.latency.min}-${proxy.latency.max}s`)
  }
  return parts.length ? parts.join(', ') : '--'
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
  const push = useToasts((state) => state.push)
  const navigate = useNavigate()
  const [pending, setPending] = useState<ProxyView>()

  // Once a minute, and immediately on arrival. `load` is stable, so this does not
  // resubscribe on every render.
  usePolling(useCallback(() => void load(), [load]))

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
        <table className="w-full text-left text-sm">
          <thead className="text-slate-500 dark:text-slate-400">
            <tr>
              <th className="py-1">Name</th>
              <th className="py-1">Upstream</th>
              <th className="py-1">Resolve</th>
              <th className="py-1">Faults</th>
              <th className="py-1">Mocks</th>
              <th className="py-1" />
            </tr>
          </thead>
          <tbody>
            {items.map(({ revision, proxy }) => (
              <tr key={proxy.name} className="border-t border-slate-200 dark:border-slate-800">
                <td className="py-1 font-medium text-slate-900 dark:text-slate-100">
                  <Link className="hover:underline" to={`/proxies/${encodeURIComponent(proxy.name)}`}>
                    {proxy.name}
                  </Link>
                </td>
                <td className="py-1 font-mono text-xs text-slate-700 dark:text-slate-300">
                  {proxy.url}
                </td>
                <td className="py-1 text-slate-700 dark:text-slate-300">{resolveOf(proxy)}</td>
                <td className="py-1 text-slate-700 dark:text-slate-300">{faults(proxy)}</td>
                <td className="py-1 text-slate-700 dark:text-slate-300">
                  {proxy.mocks?.length ?? 0}
                </td>
                <td className="py-1">
                  <div className="flex justify-end gap-2">
                    <Link
                      className="rounded border border-slate-300 px-3 py-1 text-sm text-slate-700 hover:bg-slate-100 dark:border-slate-600 dark:text-slate-200 dark:hover:bg-slate-800"
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
