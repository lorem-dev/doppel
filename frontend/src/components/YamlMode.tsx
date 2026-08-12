import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { parse, stringify } from 'yaml'

import { Button } from './Button'
import { CodeEditor } from './CodeEditor'
import { labelClass } from './Field'
import type { JsonSchema } from '../types/schema'
import type { ProxyConfig } from '../types/proxy'
import type { Trouble } from '../schema/document'
import { proxyChecker } from '../schema/document'
import { docsUrl } from '../services/docs'

/**
 * The whole proxy document as YAML, for an operator who would rather type it.
 *
 * The form is better for changing one field and worse for everything else: pasting a
 * proxy from a colleague, reordering mocks, copying a block from `main.yaml`. This is
 * the same document in the shape the configuration file has, so those are one paste.
 *
 * Kept out of the entry chunk and out of the form's: `js-yaml` and `ajv` together are
 * most of what this module weighs, and someone who never opens the YAML mode never
 * fetches them. `bundle-size.test.ts` has a budget for this chunk alone.
 *
 * What it does not preserve is comments. The document is stored as data -- JSONB in
 * the database, JSON over the API -- so a comment has nowhere to live, and saying so
 * under the editor is better than losing one silently.
 */

/** What the text is, and what is wrong with it. */
interface Reading {
  /** The document, when the text is one that could be saved. */
  document?: ProxyConfig
  /** Why it could not be, in the words the save button's tooltip wants. */
  reason?: string
  troubles: Trouble[]
}

/**
 * Read the text: parse it, check it, and say what is wrong.
 *
 * A pure function of the text and the validator, so the messages under the editor can
 * be derived during render rather than kept in state -- which is also what makes them
 * correct when the schema arrives a moment after the editor did.
 */
function read(text: string, check: ((value: unknown) => Trouble[]) | undefined): Reading {
  let parsed: unknown
  try {
    parsed = parse(text)
  } catch (caught) {
    const message = caught instanceof Error ? caught.message.split('\n')[0]! : String(caught)
    return { reason: 'this is not valid YAML', troubles: [{ where: 'document', what: message }] }
  }

  if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) {
    return {
      reason: 'this is not a proxy document',
      troubles: [{ where: 'document', what: 'a proxy is a mapping of fields' }],
    }
  }

  const troubles = check?.(parsed) ?? []
  if (troubles.length) {
    return { reason: 'the document does not match the schema', troubles }
  }

  return { document: parsed as ProxyConfig, troubles: [] }
}

export function YamlMode({
  value,
  schema,
  disabled,
  onChange,
  onBroken,
  formatToken,
}: {
  /** The document as the form holds it. Rendered to text when this mode opens. */
  value: ProxyConfig
  /** The configuration schema, for checking the document as it is typed. */
  schema: JsonSchema | undefined
  disabled?: boolean
  /** Called with the parsed document whenever the text is valid. */
  onChange: (next: ProxyConfig) => void
  /** Called when the text stops being a document the page could save. */
  onBroken: (reason: string | undefined) => void
  /**
   * Bumped by the page to ask for a reformat -- which is what pressing Save does.
   *
   * A prop rather than a ref: what the page wants is not to reach into this
   * component but to say "now would be a good time", and a value that changed is
   * exactly that.
   */
  formatToken: number
}) {
  // The text is this component's state, and the document is what leaves it. Deriving
  // the text from the document on every render would reformat what is being typed
  // after every keystroke -- the same reason `KeyValueRows` owns its rows.
  const [text, setText] = useState(() => toYaml(value))
  const check = useMemo(() => proxyChecker(schema), [schema])
  // Derived, not stored: the same text and the same validator always give the same
  // answer, and a copy in state would be a second answer that can lag behind.
  const troubles = useMemo(() => read(text, check).troubles, [text, check])

  /** Take an edit: keep the text, and tell the page what it now holds. */
  const digest = useCallback(
    (next: string) => {
      setText(next)
      const { document, reason } = read(next, check)
      onBroken(reason)
      if (document) {
        onChange(document)
      }
    },
    [onBroken, onChange, check],
  )

  /** Reformat what is there, if it can be read at all. */
  const format = useCallback(() => {
    const { document } = read(text, check)
    if (document) {
      setText(toYaml(document))
    }
  }, [text, check])

  // The page asks for a format by changing the token. The first render is not a
  // request -- the text was just written by `toYaml` and is already formatted.
  const seen = useRef(formatToken)
  useEffect(() => {
    if (formatToken !== seen.current) {
      seen.current = formatToken
      format()
    }
  }, [format, formatToken])

  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center gap-2">
        <span className={`${labelClass} mb-0 grow`}>The whole proxy, as YAML</span>
        <Button onClick={format} disabled={disabled}>
          Reformat
        </Button>
        <a
          href={docsUrl('usage/parameters/#proxies')}
          target="_blank"
          rel="noreferrer"
          className="inline-flex h-9 items-center rounded-md border border-slate-300 px-3 text-sm text-slate-700 hover:border-teal-500 hover:text-teal-700 dark:border-slate-600 dark:text-slate-200 dark:hover:border-teal-400 dark:hover:text-teal-300"
        >
          Documentation
          <span aria-hidden="true" className="ml-1 text-xs">
            &#8599;
          </span>
        </a>
      </div>

      <CodeEditor
        label="The whole proxy, as YAML"
        syntax="yaml"
        rows={24}
        value={text}
        disabled={disabled}
        onChange={digest}
      />

      {troubles.length ? (
        <ul role="alert" className="flex flex-col gap-0.5 text-xs text-red-700 dark:text-red-300">
          {troubles.map((trouble, index) => (
            <li key={index}>
              <code>{trouble.where}</code> {trouble.what}
            </li>
          ))}
        </ul>
      ) : (
        <p className="text-xs text-slate-500 dark:text-slate-400">
          Checked against this Doppel&apos;s own schema as you type. Comments are not
          kept: the document is stored as data.
        </p>
      )}
    </div>
  )
}

/**
 * The document as YAML.
 *
 * Two-space indent and no anchors, which is what a configuration file looks like;
 * key order is the document's own rather than sorted, so a round trip through this
 * mode does not rearrange a proxy nobody edited.
 *
 * `yaml` rather than `js-yaml`: same job, no dependencies, and ISC rather than a
 * dependency on argparse, whose Python-2.0 licence is outside this repository's
 * policy. `scripts/third_party.py` is what said so.
 */
function toYaml(value: ProxyConfig): string {
  return stringify(value, { indent: 2, lineWidth: 100, aliasDuplicateObjects: false })
}

export default YamlMode
