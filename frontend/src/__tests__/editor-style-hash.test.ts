// The one inline stylesheet the page is allowed to carry, held to its hash.
//
// `react-simple-code-editor` renders a `<style>` element per editor. The page's
// content security policy names it by hash rather than opening `style-src` to
// `unsafe-inline`, which means the hash has to match the library that is installed --
// and nothing in a build would say otherwise: a changed stylesheet is simply blocked,
// and the only symptom is a policy violation in an operator's console.
//
// So this recomputes it. The library is rendered rather than parsed, because what the
// policy hashes is what the browser receives.
import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import EditorImport from 'react-simple-code-editor'

/** The same interop `CodeEditor` needs: CommonJS with `exports.default`. */
const Editor = ((EditorImport as unknown as { default?: typeof EditorImport }).default ??
  EditorImport) as typeof EditorImport

/**
 * The policy the listener sends, read out of the Rust source.
 *
 * The literals inside its `concat!` rather than the whole file: the file also *talks*
 * about the policy, in a comment explaining why one hash is in it, and a check that
 * cannot tell the two apart is a check that passes for the wrong reason.
 */
const POLICY = (() => {
  const source = readFileSync(
    join(__dirname, '../../../crates/doppel-admin/src/dashboard.rs'),
    'utf8',
  )
  const block = /content-security-policy"\),\s*HeaderValue::from_static\(concat!\(([\s\S]*?)\)\)/.exec(
    source,
  )
  if (!block) {
    throw new Error('dashboard.rs no longer builds the policy from a concat! of literals')
  }
  return [...block[1]!.matchAll(/"([^"]*)"/g)].map((match) => match[1]).join('')
})()

describe("the editor's own stylesheet", () => {
  it('is the one the content security policy allows', () => {
    const markup = renderToStaticMarkup(
      createElement(Editor, { value: '', onValueChange: () => {}, highlight: (code) => code }),
    )
    const style = /<style>([\s\S]*?)<\/style>/.exec(markup)
    // The library still renders one; if it stops, the hashes below are dead weight in
    // the policy and this is where that gets noticed.
    expect(style?.[1]).toBeTruthy()

    const hash = `sha256-${createHash('sha256').update(style![1]!, 'utf8').digest('base64')}`
    expect(POLICY).toContain(hash)
  })

  it('is allowed by hash rather than by opening the policy', () => {
    // The fix that would also have silenced the console, and the one worth refusing:
    // `unsafe-inline` on `style-src` lets anything that reaches the DOM style the
    // page, which is how a defacement or an exfiltrating background image gets in.
    expect(POLICY).not.toContain('unsafe-inline')
    expect(POLICY).toContain("style-src 'self' 'sha256-")
  })
})
