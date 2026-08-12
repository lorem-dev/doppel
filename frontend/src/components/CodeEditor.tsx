import EditorImport from 'react-simple-code-editor'
import { highlight, languages } from 'prismjs'
import 'prismjs/components/prism-json'
import 'prismjs/components/prism-markup'

import './CodeEditor.css'
import { labelClass } from './Field'

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
 *
 * The grammars are what prism's package exports; the colours are in
 * `CodeEditor.css`, and both are needed -- tokenising with no palette is the state
 * this editor shipped in at first, and it looked exactly like no highlighting.
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

/**
 * One line of the editor, in `rem`.
 *
 * `text-xs` is `0.75rem`, and the wrapper carries `leading-normal` -- `1.5` -- which
 * the library's `pre` and `textarea` both inherit. Change either and change this.
 */
const LINE = 0.75 * 1.5

/** The editor's own `padding` below, in `rem`, top and bottom together. */
const PADDING = 1

export function CodeEditor({
  value,
  syntax,
  onChange,
  label,
  disabled,
  showLabel,
  rows = 6,
}: {
  value: string
  syntax: Syntax
  onChange: (next: string) => void
  label: string
  disabled?: boolean
  /**
   * Show the label rather than leaving it to a screen reader.
   *
   * A mock's editor sits under a heading that already says what it is, and its
   * label -- `alpha json` -- is a name for the assistive tree, not a caption. A
   * template's editor is the page's main control and says so on screen.
   */
  showLabel?: boolean
  /**
   * How tall the editor is when it is empty, in lines.
   *
   * It grows with its content either way; this is the floor. The default suits a
   * mock's body, which is usually a few lines. A template is a document and gets
   * more -- an editor sized to its content starts one line tall, which reads as a
   * text input for something nobody writes on one line.
   */
  rows?: number
}) {
  const grammar = syntax === 'json' ? languages.json : syntax === 'html' ? languages.markup : undefined
  // The editor renders its own textarea, so the label has to point at it by id --
  // an `aria-label` passed to the component lands on the wrapping element
  // instead, which leaves the actual input unlabelled.
  const id = `editor-${label.replace(/[^a-zA-Z0-9_-]+/g, '-')}`

  return (
    <>
      {/* Outside the box below, so showing it does not put a caption inside the
          editor's own border. */}
      <label className={showLabel ? labelClass : 'sr-only'} htmlFor={id}>
        {label}
      </label>
      <div className="mt-1 max-h-[32rem] overflow-auto rounded-md border border-slate-300 bg-white font-mono text-xs leading-normal text-slate-900 focus-within:border-teal-500 focus-within:ring-1 focus-within:ring-teal-500/40 dark:border-slate-600 dark:bg-slate-900 dark:text-slate-100">
        <Editor
          value={value}
          onValueChange={onChange}
          disabled={disabled}
          textareaId={id}
          // The floor, not the height. The library sizes the container to its
          // content, and its textarea is `height: 100%` of that container -- so a
          // minimum on the container is also a minimum on the area that takes
          // clicks, and clicking the empty space below the last line still lands
          // in the editor.
          style={{ minHeight: `${rows * LINE + PADDING}rem` }}
          // Plain text when there is no grammar for it: escaping is prism's job
          // when it highlights, and this is the branch where it does not run.
          highlight={(code) =>
            grammar
              ? highlight(code, grammar, syntax)
              : code.replace(/[&<>]/g, (c) => `&#${c.charCodeAt(0)};`)
          }
          padding={8}
          textareaClassName="outline-none"
        />
      </div>
    </>
  )
}

export default CodeEditor
