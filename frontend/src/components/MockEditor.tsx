import { Suspense, lazy } from 'react'

import { Button } from './Button'
import { Field, inputClass } from './Field'
import { KeyValueRows } from './KeyValueRows'
import { Spinner } from './Spinner'
import type { Syntax } from './CodeEditor'
import type { HttpMethod, MockConfig } from '../types/proxy'

const CodeEditor = lazy(() => import('./CodeEditor'))

const METHODS: HttpMethod[] = ['GET', 'HEAD', 'POST', 'PUT', 'PATCH', 'DELETE', 'OPTIONS']

/** Which of the three exclusive response sources a mock is using. */
type Source = 'body' | 'json' | 'template'

function sourceOf(mock: MockConfig): Source {
  if (mock.response.json !== undefined) {
    return 'json'
  }
  if (mock.response.template !== undefined) {
    return 'template'
  }
  return 'body'
}

const SYNTAX: Record<Source, Syntax> = { body: 'text', json: 'json', template: 'text' }

/**
 * One mock: what it matches, what it takes out of the request, what it answers.
 *
 * The response source is a radio because the three fields are mutually exclusive
 * in the configuration -- the server refuses a document with two of them set, so
 * the form cannot offer a state that would be refused. Switching clears the other
 * two rather than keeping them hidden: a hidden `json` that reappears on the next
 * save is a change nobody made.
 */
export function MockEditor({
  mock,
  index,
  onChange,
  onRemove,
  disabled,
  errorFor,
}: {
  mock: MockConfig
  /** This mock's position, which is how the server names it in a complaint. */
  index: number
  onChange: (next: MockConfig) => void
  onRemove: () => void
  disabled?: boolean
  errorFor: (field: string) => string | undefined
}) {
  const source = sourceOf(mock)
  // Asked for by path, so the proxy's own `name` does not answer for every mock's.
  const complaint = (field: string) => errorFor(`mocks[${index}].${field}`)

  const setResponse = (patch: Partial<MockConfig['response']>) => {
    onChange({ ...mock, response: { ...mock.response, ...patch } })
  }

  const setSource = (next: Source) => {
    const { status, headers } = mock.response
    onChange({
      ...mock,
      response: {
        status,
        headers,
        ...(next === 'body' ? { body: '' } : {}),
        ...(next === 'json' ? { json: '' } : {}),
        ...(next === 'template' ? { template: '' } : {}),
      },
    })
  }

  return (
    <div className="flex flex-col gap-2 rounded border border-slate-200 p-3 dark:border-slate-800">
      <div className="flex items-center gap-2">
        <div className="grow">
          <Field label="Mock name" error={complaint('name')}>
            <input
              className={inputClass}
              value={mock.name}
              disabled={disabled}
              onChange={(event) => onChange({ ...mock, name: event.target.value })}
            />
          </Field>
        </div>
        <Button variant="danger" disabled={disabled} onClick={onRemove}>
          Remove mock
        </Button>
      </div>

      <div className="flex gap-2">
        <Field label="Method">
          <select
            className={inputClass}
            aria-label={`${mock.name} method`}
            value={mock.request.method}
            disabled={disabled}
            onChange={(event) =>
              onChange({
                ...mock,
                request: { ...mock.request, method: event.target.value as HttpMethod },
              })
            }
          >
            {METHODS.map((method) => (
              <option key={method} value={method}>
                {method}
              </option>
            ))}
          </select>
        </Field>
        <div className="grow">
          <Field
            label="Path pattern"
            hint="A regex, matched unanchored. Named groups become template variables."
            error={complaint('url')}
          >
            <input
              className={inputClass}
              value={mock.request.url}
              disabled={disabled}
              onChange={(event) =>
                onChange({ ...mock, request: { ...mock.request, url: event.target.value } })
              }
            />
          </Field>
        </div>
      </div>

      <KeyValueRows
        label="Variables from headers"
        keyLabel="variable"
        valueLabel="header name"
        disabled={disabled}
        value={mock.request.headers}
        onChange={(next) => onChange({ ...mock, request: { ...mock.request, headers: next } })}
      />
      <KeyValueRows
        label="Variables from the query"
        keyLabel="variable"
        valueLabel="selector"
        disabled={disabled}
        value={mock.request.query}
        onChange={(next) => onChange({ ...mock, request: { ...mock.request, query: next } })}
      />
      <KeyValueRows
        label="Variables from the body"
        keyLabel="variable"
        valueLabel="selector"
        disabled={disabled}
        value={mock.request.body}
        onChange={(next) => onChange({ ...mock, request: { ...mock.request, body: next } })}
      />

      <div className="flex gap-2">
        <Field label="Status" error={complaint('status')}>
          <input
            className={inputClass}
            type="number"
            value={mock.response.status}
            disabled={disabled}
            onChange={(event) => setResponse({ status: Number(event.target.value) })}
          />
        </Field>
        <Field label="Answer with">
          <select
            className={inputClass}
            aria-label={`${mock.name} response source`}
            value={source}
            disabled={disabled}
            onChange={(event) => setSource(event.target.value as Source)}
          >
            <option value="body">Text body</option>
            <option value="json">JSON body</option>
            <option value="template">Template file</option>
          </select>
        </Field>
      </div>

      {source === 'template' ? (
        <Field
          label="Template file"
          hint="A file under this proxy's template directory, written on the Templates page."
          error={complaint('template')}
        >
          <input
            className={inputClass}
            value={mock.response.template ?? ''}
            disabled={disabled}
            onChange={(event) => setResponse({ template: event.target.value })}
          />
        </Field>
      ) : (
        <Suspense fallback={<Spinner label="Loading the editor..." />}>
          <CodeEditor
            label={`${mock.name} ${source}`}
            syntax={SYNTAX[source]}
            disabled={disabled}
            value={(source === 'json' ? mock.response.json : mock.response.body) ?? ''}
            onChange={(next) => setResponse(source === 'json' ? { json: next } : { body: next })}
          />
        </Suspense>
      )}

      <KeyValueRows
        label="Response headers"
        keyLabel="header name"
        valueLabel="template"
        disabled={disabled}
        value={mock.response.headers}
        onChange={(next) => setResponse({ headers: next })}
      />
    </div>
  )
}

/** A new mock, with the fields the server requires already filled in. */
export function emptyMock(index: number): MockConfig {
  return {
    name: `mock-${index + 1}`,
    request: { method: 'GET', url: '^/$' },
    response: { status: 200, body: '' },
  }
}
