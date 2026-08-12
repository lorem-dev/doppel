// Which grammar colours which field, and how Jinja gets into three of them.
//
// Separate from the editor component because it is the part worth testing: the
// composition below is the only place in this app where two grammars meet, and prism's
// rules about which pattern wins are not obvious enough to leave unasserted. See
// `grammars.test.ts`.

import { highlight, languages, type Grammar, type TokenObject } from 'prismjs'
import 'prismjs/components/prism-json'
import 'prismjs/components/prism-markup'
import 'prismjs/components/prism-regex'
import 'prismjs/components/prism-yaml'
// `markup-templating` is the machinery prism uses to embed a template language in a
// document; `django` is the grammar, and it registers itself as `jinja2` as well --
// which is the name the hooks below switch on.
import 'prismjs/components/prism-markup-templating'
import 'prismjs/components/prism-django'

/**
 * What a field holds.
 *
 * Three of these are templates. A mock's response body, its JSON body, a header value
 * and a template file are all rendered through Jinja, so `{{ id }}` and `{% if %}` are
 * coloured in each -- `text` means a template with no host language, which is what a
 * header value and a `.text.j2` file are.
 */
export type Syntax = 'json' | 'html' | 'text' | 'regex' | 'yaml'

/**
 * Where a Jinja expression starts and ends.
 *
 * The pattern is prism-django's own, which is what its `before-tokenize` hook uses to
 * pull expressions out of a document. It is repeated here because that hook only ever
 * hands the rest of the document to the markup grammar, and JSON is not markup.
 *
 * `inside` is the library's grammar for what lies between the delimiters: the braces,
 * the keywords, the filters, the variable names. Nothing about Jinja itself is
 * described here.
 */
/**
 * A grammar prism must have, because a component above imports it.
 *
 * A loud failure beats an editor that quietly colours nothing, which is the state this
 * whole file exists to have fixed once: prism ships its grammars as separate modules,
 * and a missing import looks exactly like a working editor with no highlighting.
 */
function required(name: string): Grammar {
  const grammar = languages[name]
  if (!grammar) {
    throw new Error(`prism has no ${name} grammar; its component is not imported`)
  }
  return grammar
}

const JINJA = {
  pattern: /\{\{[\s\S]*?\}\}|\{%[\s\S]*?%\}|\{#[\s\S]*?#\}/,
  greedy: true,
  alias: 'jinja2',
  inside: required('jinja2'),
}

/**
 * A template with no host language: expressions coloured, everything else plain.
 *
 * One token, so a word outside an expression stays a word. Handing the whole text to
 * the `jinja2` grammar instead would colour every identifier in it, because that
 * grammar describes the inside of an expression, where a bare word *is* a variable.
 */
const TEMPLATE = { jinja2: JINJA }

/**
 * JSON with Jinja in it, which is what a `.json.j2` file and a mock's `json` are.
 *
 * Two placements, because a template puts expressions in two kinds of place: inside a
 * string (`"id": "{{ id }}"`) and between the structure (`{% for %}` around an
 * element). The second is a token of its own; the first has to be `inside` the string
 * and property patterns, because prism resolves two greedy patterns competing for
 * overlapping text in favour of the one that starts earlier -- and the quote always
 * starts before the brace. A jinja token at the top level alone was silently ignored
 * for every expression inside a string, which is most of them.
 */
const JSON_TEMPLATE = (() => {
  // A private root, so the shared `languages.json` keeps its own definition: the YAML
  // editor and anything else asking prism for JSON should get JSON.
  const composed = languages.insertBefore(
    'json',
    'property',
    { jinja2: JINJA },
    { json: required('json') },
  )
  const inside = { jinja2: JINJA }
  return {
    ...composed,
    string: { ...(composed.string as TokenObject), inside },
    property: { ...(composed.property as TokenObject), inside },
  } satisfies Grammar
})()

/**
 * The highlighted markup for one field's contents.
 *
 * Returns HTML, which is what the editor's `highlight` prop wants and why every branch
 * has to escape what it does not tokenise. prism escapes as it goes; the `regex` and
 * `yaml` branches have no Jinja because a pattern and a proxy document are not
 * templates.
 */
export function colour(code: string, syntax: Syntax): string {
  switch (syntax) {
    case 'json':
      // Not the language name `jinja2`: that name is what prism's templating hook
      // switches on, and the hook hands the document to the markup grammar -- which
      // would throw the JSON away.
      return highlight(code, JSON_TEMPLATE, 'json')
    case 'html':
      // Here the hook is exactly right: the host *is* markup, so prism does the
      // embedding itself, expressions in attribute values included.
      return highlight(code, required('markup'), 'jinja2')
    case 'text':
      return highlight(code, TEMPLATE, 'text')
    case 'regex':
      return highlight(code, required('regex'), 'regex')
    case 'yaml':
      return highlight(code, required('yaml'), 'yaml')
  }
}
