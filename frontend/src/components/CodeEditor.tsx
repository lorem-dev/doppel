import EditorImport from 'react-simple-code-editor'

import './CodeEditor.css'
import { colour, type Syntax } from './grammars'
import { labelClass } from './Field'

/**
 * A textarea with syntax colouring, in its own chunk.
 *
 * prism and this editor are the only third-party code below the shell, and they are
 * here rather than hand-written because a highlighter is a solved problem and a
 * home-made one is a tokeniser to maintain. They cost roughly twelve kilobytes, which
 * is why nothing imports this module directly: the pages that need it load it lazily,
 * and `bundle-size.test.ts` fails if prism ever appears in the entry chunk.
 *
 * Which grammar colours what -- and how Jinja gets into three of them -- is in
 * `grammars.ts`, with the colours themselves in `CodeEditor.css`. All three are needed:
 * tokenising with no palette is the state this editor shipped in at first, and it
 * looked exactly like no highlighting.
 */
export type { Syntax }

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

/** The editor's own padding, in pixels, on each side. */
const PADDING_PX = 8

/** The same, in `rem`, top and bottom together. */
const PADDING = (PADDING_PX * 2) / 16

/**
 * `h-9`, the height every control on a row shares.
 *
 * A one-line editor is a control on a row -- a mock's path pattern sits beside a
 * method select -- and one two pixels short of its neighbours is the stepping this
 * project has already fixed once.
 */
const CONTROL = 2.25

/** What the editor does not decide for itself. */
interface Content {
  value: string
  syntax: Syntax
  onChange: (next: string) => void
  disabled?: boolean
  /** What an empty editor says it is for, as an input's placeholder would. */
  placeholder?: string
  /**
   * How tall the editor is when it is empty, in lines.
   *
   * It grows with its content either way; this is the floor, and it never goes below
   * the height of an ordinary control. The default suits a mock's body, which is
   * usually a few lines. A template is a document and gets more -- an editor sized to
   * its content starts one line tall, which reads as a text input for something
   * nobody writes on one line.
   */
  rows?: number
}

/**
 * How the editor's textarea gets its name, which it must have one way or another.
 *
 * Either this renders the label -- visibly, or for a screen reader only -- or the
 * caller already rendered one pointing at `id`, which is what `Field` does when it
 * holds an editor instead of an input. A caller may pass both: the rows of a map need
 * a label each *and* ids of their own, because two mocks on a page have maps with the
 * same label.
 */
type Naming =
  | { label: string; showLabel?: boolean; id?: string }
  | { id: string; label?: undefined; showLabel?: undefined }

export function CodeEditor(props: Content & Naming) {
  const { value, syntax, onChange, disabled, placeholder, rows = 6 } = props
  // The editor renders its own textarea, so the label has to point at it by id --
  // an `aria-label` passed to the component lands on the wrapping element
  // instead, which leaves the actual input unlabelled.
  const id = props.id ?? `editor-${(props.label ?? '').replace(/[^a-zA-Z0-9_-]+/g, '-')}`

  // A one-line editor is a control on a row, and its height is the row's rather than
  // its content's -- so the padding is what centres the line in it. Every other size
  // grows with its content, where a constant padding is what looks even.
  const padding = rows === 1 ? Math.round(((CONTROL - LINE) * 16) / 2) : PADDING_PX

  return (
    <>
      {/* Outside the box below, so showing it does not put a caption inside the
          editor's own border. Absent when the caller named it. */}
      {props.label === undefined ? null : (
        <label className={props.showLabel ? labelClass : 'sr-only'} htmlFor={id}>
          {props.label}
        </label>
      )}
      <div className="max-h-[32rem] overflow-auto rounded-md border border-slate-300 bg-white font-mono text-xs leading-normal text-slate-900 focus-within:border-teal-500 focus-within:ring-1 focus-within:ring-teal-500/40 dark:border-slate-600 dark:bg-slate-900 dark:text-slate-100">
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
          style={{ minHeight: `${Math.max(rows * LINE + PADDING, CONTROL)}rem` }}
          // The placeholder cannot be the textarea's: its text is transparent, which
          // is how the colouring underneath shows through -- so an empty editor draws
          // its own, in the layer that is actually visible.
          highlight={(code) =>
            code || !placeholder
              ? colour(code, syntax)
              : `<span class="text-slate-400 dark:text-slate-500">${escape(placeholder)}</span>`
          }
          padding={padding}
          textareaClassName="outline-none"
        />
      </div>
    </>
  )
}

/** For the placeholder above, which is the one string here that is not prism's. */
function escape(text: string): string {
  return text.replace(/[&<>]/g, (character) => `&#${character.charCodeAt(0)};`)
}

export default CodeEditor
