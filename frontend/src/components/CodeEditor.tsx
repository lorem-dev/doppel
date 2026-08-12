import EditorImport from 'react-simple-code-editor'
import { highlight, languages } from 'prismjs'
import 'prismjs/components/prism-json'
import 'prismjs/components/prism-markup'

/**
 * A textarea with syntax colouring, in its own chunk.
 *
 * prism and this editor are the only third-party code below the shell, and they
 * are here rather than hand-written because a highlighter is a solved problem and
 * a home-made one is a tokeniser to maintain. They cost roughly ten kilobytes,
 * which is why nothing imports this module directly: the pages that need it load
 * it lazily, and `bundle-size.test.ts` fails if prism ever appears in the entry
 * chunk.
 *
 * A template is Jinja inside JSON, HTML or text. prism's `json` and `markup`
 * grammars colour the host language; the Jinja braces are left plain rather than
 * pulling in a third grammar for the sake of two delimiters.
 */
export type Syntax = 'json' | 'html' | 'text'

/**
 * The editor component, whichever shape the bundler hands over.
 *
 * `react-simple-code-editor` is CommonJS with `exports.default`, and the two
 * builds disagree about what that means: the dev server's interop yields the
 * component, while the production build applies Node's semantics and yields the
 * whole `module.exports` -- an object, which React refuses with error #130. It
 * fails only in the built bundle, so the Playwright suite is what caught it.
 *
 * Reading `.default` when it is there is correct under both, and costs one line
 * instead of pinning a bundler's interop flag that the next major would move.
 */
const Editor = ((EditorImport as unknown as { default?: typeof EditorImport }).default ??
  EditorImport) as typeof EditorImport

export function CodeEditor({
  value,
  syntax,
  onChange,
  label,
  disabled,
}: {
  value: string
  syntax: Syntax
  onChange: (next: string) => void
  label: string
  disabled?: boolean
}) {
  const grammar = syntax === 'json' ? languages.json : syntax === 'html' ? languages.markup : undefined
  // The editor renders its own textarea, so the label has to point at it by id --
  // an `aria-label` passed to the component lands on the wrapping element
  // instead, which leaves the actual input unlabelled.
  const id = `editor-${label.replace(/[^a-zA-Z0-9_-]+/g, '-')}`

  return (
    <div className="mt-1 max-h-80 overflow-auto rounded border border-slate-300 bg-white font-mono text-xs text-slate-900 dark:border-slate-600 dark:bg-slate-900 dark:text-slate-100">
      <label className="sr-only" htmlFor={id}>
        {label}
      </label>
      <Editor
        value={value}
        onValueChange={onChange}
        disabled={disabled}
        textareaId={id}
        // Plain text when there is no grammar for it: escaping is prism's job
        // when it highlights, and this is the branch where it does not run.
        highlight={(code) =>
          grammar ? highlight(code, grammar, syntax) : code.replace(/[&<>]/g, (c) => `&#${c.charCodeAt(0)};`)
        }
        padding={8}
        textareaClassName="outline-none"
      />
    </div>
  )
}

export default CodeEditor
