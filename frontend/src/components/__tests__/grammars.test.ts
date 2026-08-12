// What gets coloured, asserted on the markup prism produces.
//
// The e2e suite measures painted pixels, which is the honest test of "is it coloured at
// all". This is the other half: which *tokens* come out, for the compositions that are
// this project's own rather than the library's. Every one of these cases was wrong at
// some point in an obvious-looking implementation.

import { colour } from '../grammars'

/** The token classes in prism's output, in order. */
function tokens(html: string): string[] {
  return [...html.matchAll(/class="token ([^"]+)"/g)].map((match) => match[1]!)
}

describe('a template with no host language', () => {
  it('colours the expression and leaves the text alone', () => {
    const html = colour('rid-{{ requestId }}', 'text')
    expect(tokens(html)).toContain('jinja2 jinja2')
    expect(tokens(html)).toContain('variable')
    // The text around it is text: `rid-` is not a variable, and neither is a word that
    // happens to be a Jinja keyword. Handing the whole string to the jinja2 grammar
    // instead colours every identifier in it.
    expect(html.startsWith('rid-')).toBe(true)
    expect(tokens(colour('and if not 42', 'text'))).toEqual([])
  })

  it('escapes what it does not tokenise', () => {
    // The result is inserted as HTML. An unescaped `<` here is markup injection into
    // the page from a field the operator typed.
    expect(colour('<b>{{ x }}</b>', 'text')).toContain('&lt;b>')
    expect(colour('a & b', 'text')).toContain('&amp;')
  })

  it('colours a statement and a comment as well as a variable', () => {
    expect(tokens(colour('{% if ready %}yes{% endif %}', 'text'))).toContain('tag keyword')
    expect(tokens(colour('{# a note #}', 'text'))).toContain('jinja2 jinja2')
  })
})

describe('JSON with Jinja in it', () => {
  it('keeps the JSON and colours an expression inside a string', () => {
    // The case that a single top-level jinja token silently missed: prism gives
    // overlapping greedy patterns to whichever starts first, and the quote starts
    // before the brace.
    const html = colour('{"id": "{{ resourceId }}", "n": 1}', 'json')
    const classes = tokens(html)
    expect(classes).toContain('property')
    expect(classes).toContain('number')
    expect(classes).toContain('jinja2 jinja2')
    expect(classes).toContain('variable')
  })

  it('colours an expression between the structure', () => {
    const classes = tokens(colour('{"a": 1{% if x %}, "b": 2{% endif %}}', 'json'))
    expect(classes).toContain('tag keyword')
    expect(classes).toContain('punctuation')
  })

  it('leaves plain JSON as plain JSON', () => {
    const classes = tokens(colour('{"a": [1, true, null]}', 'json'))
    expect(classes).toContain('boolean')
    expect(classes.some((name) => name.includes('jinja2'))).toBe(false)
  })
})

describe('the two that are not templates', () => {
  it('colours a pattern as a pattern', () => {
    const classes = tokens(colour('^/api/(?P<id>\\d+)$', 'regex'))
    expect(classes).toContain('anchor function')
    expect(classes.some((name) => name.includes('jinja2'))).toBe(false)
  })

  it('colours a document as YAML, braces and all', () => {
    // A proxy document is not a template: `{{` in one is two brackets, and colouring
    // it as an expression would say otherwise.
    const classes = tokens(colour('name: alpha\ntimeout: 30\n', 'yaml'))
    expect(classes).toContain('key atrule')
    expect(classes.some((name) => name.includes('jinja2'))).toBe(false)
  })
})
