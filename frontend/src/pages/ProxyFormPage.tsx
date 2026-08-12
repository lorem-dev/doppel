import { useCallback, useEffect, useState } from 'react'
import { useNavigate, useParams } from 'react-router'

import { Banner } from '../components/Banner'
import { Button } from '../components/Button'
import { Field, controlClass, selectFullClass } from '../components/Field'
import { KeyValueRows } from '../components/KeyValueRows'
import { MockEditor, emptyMock } from '../components/MockEditor'
import { Section } from '../components/Section'
import { Spinner } from '../components/Spinner'
import type { ProxyConfig } from '../types/proxy'
import { ApiError, complaintAbout } from '../types/error'
import { complain, numericAttrs } from '../schema/rules'
import { useMay } from '../store/access'
import { useRule, useValueRule } from '../store/schema'
import { useToasts } from '../store/toast'
import { createProxy, readProxy, updateProxy } from '../services/proxies'

/** A new proxy, with the two fields the server requires. */
const BLANK: ProxyConfig = { name: '', type: 'http', url: '' }

/**
 * What each folded section says about itself.
 *
 * A row of four identical closed headings would make the operator open all of them
 * to find out which one holds the setting they came for.
 */
function forwardingSummary(proxy: ProxyConfig): string {
  const parts = [proxy.resolve?.type === 'header' ? 'by header' : 'default proxy']
  if (proxy.timeout !== undefined) {
    parts.push(`${proxy.timeout}s timeout`)
  }
  const headers = Object.keys(proxy.headers ?? {}).length
  if (headers) {
    parts.push(`${headers} header${headers === 1 ? '' : 's'}`)
  }
  return parts.join(', ')
}

function faultSummary(proxy: ProxyConfig): string {
  const parts: string[] = []
  if (proxy.replace !== undefined) {
    parts.push('replace')
  }
  if (proxy.loss) {
    parts.push('loss')
  }
  if (proxy.latency) {
    parts.push('latency')
  }
  if (proxy.rewrite_redirects === false) {
    parts.push('redirects kept')
  }
  return parts.length ? parts.join(', ') : 'none'
}

function accessSummary(proxy: ProxyConfig): string {
  const set = Object.keys(proxy.access ?? {})
  return set.length ? set.join(', ') : 'inherited'
}

/**
 * An optional number field: empty means absent, not zero.
 *
 * Anything that is not a number is also absent. `Number('abc')` is `NaN`, which
 * `JSON.stringify` writes as `null` -- so without this the server would refuse the
 * document with a complaint about a null where the operator had typed a word.
 */
function numberOr(value: string): number | undefined {
  const trimmed = value.trim()
  if (trimmed === '') {
    return undefined
  }
  const parsed = Number(trimmed)
  return Number.isFinite(parsed) ? parsed : undefined
}

/**
 * Create or edit one proxy, field by field.
 *
 * The form covers every field of `ProxyConfig`, and `schema-drift.test.ts` is
 * what keeps that true: it reads `doppel-config.schema.json` and fails when the
 * configuration format grows a field this file does not mention.
 *
 * Nothing here re-implements the configuration's rules. The bounds it checks as
 * someone types -- a pattern, a length, a range -- are read out of the schema the
 * server serves, so there is one statement of each rule and the page cannot hold a
 * stale or laxer copy of it. Anything that needs more than one field is the
 * server's answer on save, mapped back onto the field it is about.
 */
export default function ProxyFormPage() {
  const { name } = useParams()
  const editing = name !== undefined
  const navigate = useNavigate()
  const may = useMay()
  const rule = useRule()
  const valueRule = useValueRule()
  const push = useToasts((state) => state.push)

  const [draft, setDraft] = useState<ProxyConfig>(BLANK)
  const [revision, setRevision] = useState<string>()
  const [loading, setLoading] = useState(editing)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<ApiError>()

  useEffect(() => {
    if (!editing) {
      return
    }
    readProxy(name)
      .then((view) => {
        setDraft(view.proxy)
        setRevision(view.revision)
        setError(undefined)
      })
      .catch((caught: ApiError) => setError(caught))
      .finally(() => setLoading(false))
  }, [editing, name])

  const allowed = editing ? may('update', name) : may('create')

  /**
   * The server's complaint about one field, if it made one.
   *
   * The form asks about a field it knows; nothing here reads paths out of the
   * message. A complaint that matches no field stays in the banner rather than
   * being dropped -- which is where all of them end up when the refusal is about
   * the document as a whole.
   */
  const errorFor = useCallback(
    (field: string): string | undefined =>
      error ? complaintAbout(error, field) : undefined,
    [error],
  )

  /**
   * What the schema says is wrong with this field's current value.
   *
   * Shown in preference to the server's complaint about the same field: it is about
   * the text that is there now, while the server's is about the text that was there
   * when the save was refused.
   */
  const complaintFor = useCallback(
    (path: string, value: unknown): string | undefined => complain(rule(path), value),
    [rule],
  )

  const save = () => {
    setSaving(true)
    const done = (message: string) => {
      push('done', message)
      void navigate('/')
    }
    const failed = (caught: ApiError) => {
      setError(caught)
      setSaving(false)
    }

    if (editing) {
      if (!revision) {
        // Unreachable through the UI -- the revision arrives with the document --
        // and worth refusing rather than sending a blind write the server would
        // answer 428 to.
        failed(new ApiError(0, 'REVISION_REQUIRED', 'this proxy was never loaded'))
        return
      }
      updateProxy(name, revision, draft)
        .then(() => done(`Updated ${draft.name}`))
        .catch(failed)
      return
    }
    createProxy(draft)
      .then(() => done(`Created ${draft.name}`))
      .catch(failed)
  }

  if (loading) {
    return <Spinner label="Reading the proxy..." />
  }

  const patch = (fields: Partial<ProxyConfig>) => setDraft({ ...draft, ...fields })

  return (
    <section className="flex flex-col gap-4">
      <div className="flex items-center gap-3">
        <h2 className="text-base font-semibold text-slate-900 dark:text-slate-100">
          {editing ? `Edit ${name}` : 'Add a proxy'}
        </h2>
      </div>

      {error ? (
        <Banner kind="error">
          {error.isStale
            ? `${error.message}\nThis proxy changed since it was loaded. Reload the page to see the current document.`
            : error.message}
        </Banner>
      ) : null}

      <Field
        label="Name"
        hint="Letters, digits, - and _, 2 to 32 characters."
        error={complaintFor('name', draft.name) ?? errorFor('name')}
      >
        <input
          className={controlClass}
          value={draft.name}
          disabled={!allowed || saving}
          onChange={(event) => patch({ name: event.target.value })}
        />
      </Field>

      <Field label="Type" hint="Only http. tcp is not implemented and the server refuses it.">
        <input className={controlClass} value={draft.type} disabled readOnly aria-label="Type" />
      </Field>

      <Field label="Upstream URL" error={errorFor('url')}>
        <input
          className={controlClass}
          value={draft.url}
          disabled={!allowed || saving}
          onChange={(event) => patch({ url: event.target.value })}
        />
      </Field>

      <Section
        title="Forwarding"
        summary={forwardingSummary(draft)}
      >
      <div className="flex gap-3">
        <Field
          label="Timeout (seconds)"
          hint="Absent means no per-proxy timeout."
          error={complaintFor('timeout', draft.timeout) ?? errorFor('timeout')}
        >
          <input
            className={controlClass}
            type="number"
            {...numericAttrs(rule('timeout'))}
            value={draft.timeout ?? ''}
            disabled={!allowed || saving}
            onChange={(event) => patch({ timeout: numberOr(event.target.value) })}
          />
        </Field>
        <Field
          label="Body limit"
          hint="Bytes, or 4Mi."
          error={complaintFor('body_limit', draft.body_limit) ?? errorFor('body_limit')}
        >
          <input
            className={controlClass}
            value={draft.body_limit ?? ''}
            disabled={!allowed || saving}
            onChange={(event) =>
              patch({ body_limit: event.target.value.trim() === '' ? undefined : event.target.value })
            }
          />
        </Field>
      </div>

      <div className="flex gap-2">
        <Field label="Resolve by">
          <select
            className={selectFullClass}
            aria-label="Resolve by"
            value={draft.resolve?.type ?? 'default'}
            disabled={!allowed || saving}
            onChange={(event) =>
              patch({
                resolve:
                  event.target.value === 'header'
                    ? { type: 'header', header: draft.resolve?.header ?? 'X-Proxy-Name' }
                    : { type: 'default' },
              })
            }
          >
            <option value="default">The default proxy</option>
            <option value="header">A request header</option>
          </select>
        </Field>
        {draft.resolve?.type === 'header' ? (
          <div className="grow">
            <Field label="Header name" error={errorFor('header')}>
              <input
                className={controlClass}
                value={draft.resolve.header ?? ''}
                disabled={!allowed || saving}
                onChange={(event) => patch({ resolve: { type: 'header', header: event.target.value } })}
              />
            </Field>
          </div>
        ) : null}
      </div>

      <KeyValueRows
        label="Headers sent upstream"
        keyLabel="header name"
        valueLabel="value"
        disabled={!allowed || saving}
        valueRule={valueRule('headers')}
        value={draft.headers}
        onChange={(next) => patch({ headers: next })}
      />
      </Section>

      <Section title="Faults" summary={faultSummary(draft)}>
        <div className="flex gap-3">
          <Field
            label="Replace"
            hint="A fraction: 0.25 replaces a quarter of matching requests with a mock."
            error={complaintFor('replace', draft.replace) ?? errorFor('replace')}
          >
            <input
              className={controlClass}
              type="number"
              step="0.01"
              {...numericAttrs(rule('replace'))}
              value={draft.replace ?? ''}
              disabled={!allowed || saving}
              onChange={(event) => patch({ replace: numberOr(event.target.value) })}
            />
          </Field>
          <Field label="Rewrite redirects" hint="On by default: a Location pointing upstream is pointed back here.">
            <select
              className={selectFullClass}
              aria-label="Rewrite redirects"
              value={draft.rewrite_redirects === undefined ? 'default' : String(draft.rewrite_redirects)}
              disabled={!allowed || saving}
              onChange={(event) =>
                patch({
                  rewrite_redirects:
                    event.target.value === 'default' ? undefined : event.target.value === 'true',
                })
              }
            >
              <option value="default">Default (on)</option>
              <option value="true">On</option>
              <option value="false">Off</option>
            </select>
          </Field>
        </div>

        <div className="flex gap-3">
          <Field
            label="Loss rate"
            hint="A fraction. Leave empty for none."
            error={complaintFor('loss.percentage', draft.loss?.percentage) ?? errorFor('percentage')}
          >
            <input
              className={controlClass}
              type="number"
              step="0.01"
              {...numericAttrs(rule('loss.percentage'))}
              value={draft.loss?.percentage ?? ''}
              disabled={!allowed || saving}
              onChange={(event) => {
                const percentage = numberOr(event.target.value)
                patch({
                  loss:
                    percentage === undefined
                      ? undefined
                      : { percentage, status: draft.loss?.status ?? 503 },
                })
              }}
            />
          </Field>
          <Field
            label="Loss status"
            error={complaintFor('loss.status', draft.loss?.status) ?? errorFor('status')}
          >
            <input
              className={controlClass}
              type="number"
              {...numericAttrs(rule('loss.status'))}
              value={draft.loss?.status ?? ''}
              disabled={!allowed || saving || !draft.loss}
              onChange={(event) =>
                patch({
                  loss: draft.loss
                    ? { ...draft.loss, status: Number(event.target.value) }
                    : undefined,
                })
              }
            />
          </Field>
        </div>

        <div className="flex gap-3">
          <Field
            label="Latency rate"
            hint="A fraction of requests to delay."
            error={complaintFor('latency.percentage', draft.latency?.percentage)}
          >
            <input
              className={controlClass}
              type="number"
              step="0.01"
              {...numericAttrs(rule('latency.percentage'))}
              value={draft.latency?.percentage ?? ''}
              disabled={!allowed || saving}
              onChange={(event) => {
                const percentage = numberOr(event.target.value)
                patch({
                  latency:
                    percentage === undefined
                      ? undefined
                      : {
                          percentage,
                          min: draft.latency?.min ?? 0,
                          max: draft.latency?.max ?? 1,
                        },
                })
              }}
            />
          </Field>
          <Field
            label="Minimum (seconds)"
            error={complaintFor('latency.min', draft.latency?.min) ?? errorFor('min')}
          >
            <input
              className={controlClass}
              type="number"
              step="0.1"
              {...numericAttrs(rule('latency.min'))}
              value={draft.latency?.min ?? ''}
              disabled={!allowed || saving || !draft.latency}
              onChange={(event) =>
                patch({
                  latency: draft.latency
                    ? { ...draft.latency, min: Number(event.target.value) }
                    : undefined,
                })
              }
            />
          </Field>
          <Field
            label="Maximum (seconds)"
            error={complaintFor('latency.max', draft.latency?.max) ?? errorFor('max')}
          >
            <input
              className={controlClass}
              type="number"
              step="0.1"
              {...numericAttrs(rule('latency.max'))}
              value={draft.latency?.max ?? ''}
              disabled={!allowed || saving || !draft.latency}
              onChange={(event) =>
                patch({
                  latency: draft.latency
                    ? { ...draft.latency, max: Number(event.target.value) }
                    : undefined,
                })
              }
            />
          </Field>
        </div>
      </Section>

      <Section title="Access overrides" summary={accessSummary(draft)}>
        <p className="text-xs text-slate-500 dark:text-slate-400">
          Empty means this proxy follows <code>admin.access</code>. A name must be one
          <code> admin.groups</code> allows.
        </p>
        {(['read', 'update', 'delete', 'upload'] as const).map((action) => {
          const current = draft.access?.[action]
          const value = current === undefined ? '' : current === 'public' ? 'public' : current.join(', ')
          return (
            <Field key={action} label={action} error={errorFor(action)}>
              <input
                className={controlClass}
                placeholder="inherit"
                value={value}
                disabled={!allowed || saving}
                onChange={(event) => {
                  const text = event.target.value.trim()
                  const access = { ...draft.access }
                  if (text === '') {
                    delete access[action]
                  } else if (text === 'public') {
                    access[action] = 'public'
                  } else {
                    access[action] = text.split(',').map((part) => part.trim()).filter(Boolean)
                  }
                  patch({ access: Object.keys(access).length ? access : undefined })
                }}
              />
            </Field>
          )
        })}
      </Section>

      <Section
        title="Mocks"
        summary={
          (draft.mocks?.length ?? 0) === 0 ? 'none' : `${draft.mocks?.length ?? 0}`
        }
      >
        {(draft.mocks ?? []).map((mock, index) => (
          <MockEditor
            key={index}
            mock={mock}
            index={index}
            disabled={!allowed || saving}
            errorFor={errorFor}
            rule={rule}
            valueRule={valueRule}
            onChange={(next) => {
              const mocks = [...(draft.mocks ?? [])]
              mocks[index] = next
              patch({ mocks })
            }}
            onRemove={() => {
              const mocks = (draft.mocks ?? []).filter((_, at) => at !== index)
              patch({ mocks: mocks.length ? mocks : undefined })
            }}
          />
        ))}
        <Button
          disabled={!allowed || saving}
          onClick={() => patch({ mocks: [...(draft.mocks ?? []), emptyMock((draft.mocks ?? []).length)] })}
        >
          Add a mock
        </Button>
      </Section>

      {/*
        Sticky, because the form is long enough that the button was a scroll away
        from whatever was just typed. `bottom-0` inside the page's scroll container,
        with the page's own background under it -- a transparent bar over a table of
        inputs is unreadable at exactly the moment it matters.
      */}
      <div className="sticky bottom-0 -mx-4 mt-2 flex items-center gap-2 border-t border-slate-200 bg-white px-4 py-3 dark:border-slate-800 dark:bg-slate-950">
        <Button
          variant="primary"
          onClick={save}
          disabled={!allowed || saving || !draft.name.trim() || !draft.url.trim()}
          reason={editing ? 'This token may not change this proxy' : 'This token may not create proxies'}
        >
          {saving ? 'Saving...' : editing ? 'Save changes' : 'Create proxy'}
        </Button>
        <Button onClick={() => void navigate('/')} disabled={saving}>
          Cancel
        </Button>
        {revision ? (
          <span className="ml-auto font-mono text-xs text-slate-500 dark:text-slate-400">
            {revision}
          </span>
        ) : null}
      </div>
    </section>
  )
}
