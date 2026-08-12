import { Suspense, lazy, useState } from 'react'

import { Button } from './Button'
import { Field, controlClass, selectFullClass } from './Field'
import { KeyValueRows } from './KeyValueRows'
import { Section } from './Section'
import { Spinner } from './Spinner'
import type { Syntax } from './CodeEditor'
import type { HttpMethod, MockConfig } from '../types/proxy'
import type { Rule } from '../schema/rules'
import { complain, numericAttrs } from '../schema/rules'

const CodeEditor = lazy(() => import('./CodeEditor'))

const METHODS: HttpMethod[] = ['GET', 'HEAD', 'POST', 'PUT', 'PATCH', 'DELETE', 'OPTIONS']

/**
 * Which of the three exclusive response sources a mock is using.
 *
 * `template` is not something this page can choose. A template file is uploaded
 * through the admin API, and managing files was a poor fit for a form over a
 * document -- so the dashboard reads that source, shows which file a mock answers
 * with, and lets an operator move the mock off it. What it never does is name a file
 * that would have to exist.
 */
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

/** What the "Answer with" select offers, and what each is called. */
const SOURCES: Array<{ value: Source; label: string }> = [
  { value: 'body', label: 'Text body' },
  { value: 'json', label: 'JSON body' },
]

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
  rule,
  valueRule,
}: {
  mock: MockConfig
  /** This mock's position, which is how the server names it in a complaint. */
  index: number
  onChange: (next: MockConfig) => void
  onRemove: () => void
  disabled?: boolean
  errorFor: (field: string) => string | undefined
  /**
   * The schema's bounds on a field of a mock, by path within a proxy document.
   *
   * Passed in rather than read from the store here: every mock in a form would
   * otherwise subscribe to the schema separately, and the form already has it.
   */
  rule: (path: string) => Rule | undefined
  valueRule: (path: string) => Rule | undefined
}) {
  const source = sourceOf(mock)
  // Asked for by path, so the proxy's own `name` does not answer for every mock's.
  const complaint = (field: string) => errorFor(`mocks[${index}].${field}`)
  // The schema describes one mock, so the path has no index in it: every mock in
  // the list is the same shape.
  const bounds = (field: string) => rule(`mocks[].${field}`)
  // The pattern editor is named by the field around it, so both need the same id --
  // and every mock on the page needs its own.
  const patternId = `mock-${index}-pattern`
  // The template this mock arrived with, if any: what the document said when it was
  // read, which is what makes switching away from a template undoable while the page
  // is open. State with an initializer rather than a ref, because it is read while
  // rendering -- a ref read during a render is the thing `react-hooks/refs` is for.
  const [configured] = useState(mock.response.template)
  const checked = (field: string, value: unknown) => complain(bounds(field), value)

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
        // Back to the file it came with, since the page has no way to name another.
        ...(next === 'template' ? { template: configured ?? '' } : {}),
      },
    })
  }

  return (
    <div className="flex flex-col gap-3 rounded-md border border-slate-200 p-3 dark:border-slate-800">
      <div className="flex items-end gap-2">
        <div className="grow">
          <Field
            label="Mock name"
            info="mocks[].name"
            error={checked('name', mock.name) ?? complaint('name')}
          >
            <input
              className={controlClass}
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

      <div className="flex gap-3">
        <Field label="Method" info="mocks[].request.method">
          <select
            className={selectFullClass}
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
          {/*
            A code editor rather than an input, for one line of regex. The pattern is
            the field an operator gets wrong most often, and colouring the groups,
            classes and quantifiers is what makes a stray bracket visible before the
            server compiles it. `Field` names it by id, because the editor renders its
            own textarea and a label cannot wrap one.
          */}
          <Field
            label="Path pattern"
            info="mocks[].request.url"
            hint="A regex, matched unanchored. Named groups become template variables."
            error={checked('request.url', mock.request.url) ?? complaint('url')}
            htmlFor={patternId}
          >
            <Suspense fallback={<Spinner label="Loading the editor..." />}>
              <CodeEditor
                id={patternId}
                syntax="regex"
                rows={1}
                disabled={disabled}
                value={mock.request.url}
                onChange={(next) =>
                  onChange({ ...mock, request: { ...mock.request, url: next } })
                }
              />
            </Suspense>
          </Field>
        </div>
      </div>

      <Section title="Variables from the request" summary={variableSummary(mock)}>
      <KeyValueRows
        label="Variables from headers"
        keyLabel="variable"
        valueLabel="header name"
        info="mocks[].request.headers"
        disabled={disabled}
        valueRule={valueRule('mocks[].request.headers')}
        value={mock.request.headers}
        onChange={(next) => onChange({ ...mock, request: { ...mock.request, headers: next } })}
      />
      <KeyValueRows
        label="Variables from the query"
        keyLabel="variable"
        valueLabel="selector"
        info="mocks[].request.query"
        disabled={disabled}
        valueRule={valueRule('mocks[].request.query')}
        value={mock.request.query}
        onChange={(next) => onChange({ ...mock, request: { ...mock.request, query: next } })}
      />
      <KeyValueRows
        label="Variables from the body"
        keyLabel="variable"
        valueLabel="selector"
        info="mocks[].request.body"
        disabled={disabled}
        valueRule={valueRule('mocks[].request.body')}
        value={mock.request.body}
        onChange={(next) => onChange({ ...mock, request: { ...mock.request, body: next } })}
      />
      </Section>

      <div className="flex gap-3">
        <Field
          label="Status"
          info="mocks[].response.status"
          error={checked('response.status', mock.response.status) ?? complaint('status')}
        >
          <input
            className={controlClass}
            type="number"
            {...numericAttrs(bounds('response.status'))}
            value={mock.response.status}
            disabled={disabled}
            onChange={(event) => setResponse({ status: Number(event.target.value) })}
          />
        </Field>
        <Field label="Answer with">
          <select
            className={selectFullClass}
            aria-label={`${mock.name} response source`}
            value={source}
            disabled={disabled}
            onChange={(event) => setSource(event.target.value as Source)}
          >
            {SOURCES.map(({ value, label }) => (
              <option key={value} value={value}>
                {label}
              </option>
            ))}
            {/*
              Offered only to a mock that arrived with one, and it stays offered while
              the page is open so switching away is undoable. A mock that never had a
              template cannot be given one here.
            */}
            {configured === undefined ? null : (
              <option value="template">Template file (from the configuration)</option>
            )}
          </select>
        </Field>
      </div>

      {source === 'template' ? (
        <Field
          label="Template file"
          info="mocks[].response.template"
          hint="Set in the configuration. Template files are uploaded through the admin API, not from here -- change the answer above to edit it as a body."
          error={complaint('template')}
        >
          <input
            className={controlClass}
            value={mock.response.template ?? ''}
            disabled
            readOnly
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
        info="mocks[].response.headers"
        valueSyntax="text"
        disabled={disabled}
        valueRule={valueRule('mocks[].response.headers')}
        value={mock.response.headers}
        onChange={(next) => setResponse({ headers: next })}
      />
    </div>
  )
}

/** How many variables a mock extracts, for the folded summary. */
function variableSummary(mock: MockConfig): string {
  const count =
    Object.keys(mock.request.headers ?? {}).length +
    Object.keys(mock.request.query ?? {}).length +
    Object.keys(mock.request.body ?? {}).length
  return count === 0 ? 'none' : `${count}`
}

/** A new mock, with the fields the server requires already filled in. */
export function emptyMock(index: number): MockConfig {
  return {
    name: `mock-${index + 1}`,
    request: { method: 'GET', url: '^/$' },
    response: { status: 200, body: '' },
  }
}
