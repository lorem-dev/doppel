import { useCallback, useEffect, useState } from 'react'

import { Banner } from '../components/Banner'
import { Button } from '../components/Button'
import { Spinner } from '../components/Spinner'
import type { ReloadReport, Status } from '../types/api'
import { ApiError } from '../types/error'
import { useMay } from '../store/access'
import { useToasts } from '../store/toast'
import { fetchStatus, reload } from '../services/status'

/**
 * What the process is serving, and the button that makes it re-read the store.
 *
 * `/status` is unauthenticated by design, so this page works with no token -- but
 * the reload button is a write in every sense that matters, so it is gated on
 * `update` like one.
 */
export default function StatusPage() {
  const [status, setStatus] = useState<Status>()
  const [error, setError] = useState<ApiError>()
  const [report, setReport] = useState<ReloadReport>()
  const may = useMay()
  const push = useToasts((state) => state.push)

  const load = useCallback(() => {
    fetchStatus()
      .then((next) => {
        setStatus(next)
        setError(undefined)
      })
      .catch((caught: ApiError) => setError(caught))
  }, [])

  useEffect(load, [load])

  const onReload = () => {
    reload()
      .then((next) => {
        setReport(next)
        push('done', `Reloaded at revision ${next.revision}`)
        load()
      })
      .catch((caught: ApiError) => push('failed', caught.message))
  }

  return (
    <section className="flex flex-col gap-4">
      <div className="flex items-center gap-3">
        <h2 className="text-base font-semibold text-slate-900 dark:text-slate-100">Status</h2>
        <Button
          variant="primary"
          onClick={onReload}
          disabled={!may('update')}
          reason="This token may not reload the configuration"
        >
          Reload configuration
        </Button>
      </div>

      {error ? <Banner kind="error">{error.message}</Banner> : null}

      {report ? (
        <Banner kind="note">
          {`Revision ${report.revision}, ${report.proxies} proxies.`}
          {report.unapplied?.length
            ? `\nNeeds a restart to take effect: ${report.unapplied.join(', ')}`
            : ''}
        </Banner>
      ) : null}

      {!status && !error ? <Spinner label="Reading status..." /> : null}

      {status ? (
        <dl className="grid grid-cols-[max-content_1fr] gap-x-4 gap-y-1 text-sm">
          <dt className="text-slate-500 dark:text-slate-400">Uptime</dt>
          <dd className="text-slate-900 dark:text-slate-100">{status.uptime_seconds}s</dd>
          <dt className="text-slate-500 dark:text-slate-400">Revision</dt>
          <dd className="font-mono text-slate-900 dark:text-slate-100">{status.revision}</dd>
          <dt className="text-slate-500 dark:text-slate-400">Proxies</dt>
          <dd className="text-slate-900 dark:text-slate-100">{status.proxies.length}</dd>
        </dl>
      ) : null}

      {status?.proxies.length ? (
        <table className="w-full text-left text-sm">
          <thead className="text-slate-500 dark:text-slate-400">
            <tr>
              <th className="py-1">Proxy</th>
              <th className="py-1">Upstream</th>
              <th className="py-1">Resolve</th>
              <th className="py-1">Mocks</th>
            </tr>
          </thead>
          <tbody>
            {status.proxies.map((proxy) => (
              <tr key={proxy.name} className="border-t border-slate-200 dark:border-slate-800">
                <td className="py-1 font-medium text-slate-900 dark:text-slate-100">{proxy.name}</td>
                <td className="py-1 font-mono text-xs text-slate-700 dark:text-slate-300">
                  {proxy.upstream}
                </td>
                <td className="py-1 text-slate-700 dark:text-slate-300">{proxy.resolve}</td>
                <td className="py-1 text-slate-700 dark:text-slate-300">{proxy.mocks}</td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : null}
    </section>
  )
}
