import { Suspense, lazy, useCallback, useEffect, useState } from 'react'
import { Link, useParams } from 'react-router'

import { Banner } from '../components/Banner'
import { Button } from '../components/Button'
import { Field, controlClass, selectFullClass } from '../components/Field'
import { Spinner } from '../components/Spinner'
import type { Syntax } from '../components/CodeEditor'
import type { TemplateEntry } from '../types/api'
import { ApiError } from '../types/error'
import { useMay } from '../store/access'
import { useToasts } from '../store/toast'
import { deleteTemplate, listTemplates, putTemplate } from '../services/templates'

const CodeEditor = lazy(() => import('../components/CodeEditor'))

/**
 * The three extensions a template file may have, and the grammar each implies.
 *
 * A template renders through Jinja whatever it is called; the extension says what
 * the *output* is, which is what decides how the editor colours it and what the
 * mock referencing it may claim to serve.
 */
const KINDS: Array<{ extension: string; syntax: Syntax; label: string }> = [
  { extension: 'json.j2', syntax: 'json', label: 'JSON' },
  { extension: 'html.j2', syntax: 'html', label: 'HTML' },
  { extension: 'text.j2', syntax: 'text', label: 'Text' },
]

/**
 * A proxy's template files, authored here rather than uploaded.
 *
 * There is deliberately no file picker: these are short templates, and a second
 * way in -- pick a file, watch it be read, hope the encoding survived -- buys
 * nothing over a textarea whose contents are exactly what the server will store.
 */
export default function TemplatesPage() {
  const { name = '' } = useParams()
  const [files, setFiles] = useState<TemplateEntry[]>([])
  const [error, setError] = useState<ApiError>()
  const [loading, setLoading] = useState(true)
  const [stem, setStem] = useState('')
  const [kind, setKind] = useState(KINDS[0]!)
  const [content, setContent] = useState('')
  const may = useMay()
  const push = useToasts((state) => state.push)

  const load = useCallback(() => {
    listTemplates(name)
      .then((next) => {
        setFiles(next)
        setError(undefined)
      })
      .catch((caught: ApiError) => setError(caught))
      .finally(() => setLoading(false))
  }, [name])

  useEffect(load, [load])

  const file = `${stem}.${kind.extension}`
  const allowed = may('upload', name)

  const onSave = () => {
    putTemplate(name, file, content)
      .then(() => {
        push('done', `Wrote ${file}`)
        load()
      })
      .catch((caught: ApiError) => push('failed', caught.message))
  }

  return (
    <section className="flex flex-col gap-4">
      <div className="flex items-center gap-3">
        <h2 className="text-base font-semibold text-slate-900 dark:text-slate-100">
          Templates for {name}
        </h2>
        <Link className="text-sm text-teal-700 hover:underline dark:text-teal-300" to="/">
          Back to proxies
        </Link>
      </div>

      {error ? <Banner kind="error">{error.message}</Banner> : null}
      {loading ? <Spinner label="Reading the template list..." /> : null}

      {files.length ? (
        <table className="w-full text-left text-sm">
          <thead className="text-slate-500 dark:text-slate-400">
            <tr>
              <th className="py-1">File</th>
              <th className="py-1">Bytes</th>
              <th className="py-1" />
            </tr>
          </thead>
          <tbody>
            {files.map((entry) => (
              <tr key={entry.name} className="border-t border-slate-200 dark:border-slate-800">
                <td className="py-1 font-mono text-xs text-slate-900 dark:text-slate-100">
                  {entry.name}
                </td>
                <td className="py-1 text-slate-700 dark:text-slate-300">{entry.size}</td>
                <td className="py-1 text-right">
                  <Button
                    variant="danger"
                    disabled={!allowed}
                    reason="This token may not change this proxy's templates"
                    onClick={() =>
                      void deleteTemplate(name, entry.name)
                        .then(() => {
                          push('done', `Deleted ${entry.name}`)
                          load()
                        })
                        .catch((caught: ApiError) => push('failed', caught.message))
                    }
                  >
                    Delete
                  </Button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : null}

      <div className="flex flex-col gap-2 rounded border border-slate-200 p-3 dark:border-slate-800">
        <h3 className="text-sm font-semibold text-slate-900 dark:text-slate-100">Write a template</h3>
        <div className="flex items-end gap-2">
          <div className="grow">
            <Field label="File name" hint="Letters, digits, - and _. The extension is chosen beside it.">
              <input
                className={controlClass}
                value={stem}
                disabled={!allowed}
                onChange={(event) => setStem(event.target.value)}
              />
            </Field>
          </div>
          <Field label="Kind">
            <select
              className={selectFullClass}
              aria-label="Kind"
              value={kind.extension}
              disabled={!allowed}
              onChange={(event) =>
                setKind(KINDS.find((entry) => entry.extension === event.target.value) ?? KINDS[0]!)
              }
            >
              {KINDS.map((entry) => (
                <option key={entry.extension} value={entry.extension}>
                  {entry.label}
                </option>
              ))}
            </select>
          </Field>
        </div>

        <Suspense fallback={<Spinner label="Loading the editor..." />}>
          <CodeEditor
            label={`Contents of ${file}`}
            syntax={kind.syntax}
            value={content}
            disabled={!allowed}
            onChange={setContent}
          />
        </Suspense>

        <div className="flex items-center gap-2">
          <Button
            variant="primary"
            onClick={onSave}
            disabled={!allowed || !stem.trim() || !content}
            reason="This token may not change this proxy's templates"
          >
            Save {stem.trim() ? file : 'template'}
          </Button>
          <span className="text-xs text-slate-500 dark:text-slate-400">
            A file has to be declared by one of this proxy&apos;s mocks first: the server refuses
            an upload nothing refers to.
          </span>
        </div>
      </div>
    </section>
  )
}
