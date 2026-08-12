// The static payload has a budget, and the budget has a test.
//
// "As small as possible" is a preference until something fails when it is
// broken. Measured gzipped, because that is what crosses the wire, and split by
// role: the entry chunk is what every visitor pays for, a page chunk is paid
// only by whoever opens that page, and the editor chunk exists precisely so that
// prism is not paid for by someone who only lists proxies.
//
// The numbers are the measured sizes plus about ten percent. They are recorded
// beside each budget so that a jump shows up in the diff as a number somebody
// chose rather than as a limit quietly raised to make a build pass.
import { gzipSync } from 'node:zlib'
import { existsSync, readFileSync, readdirSync } from 'node:fs'
import { join } from 'node:path'

const DIST = join(__dirname, '../../dist')
const ASSETS = join(DIST, 'assets')

/** Gzipped kilobytes of one built file. */
const gzippedKb = (file: string): number =>
  gzipSync(readFileSync(join(ASSETS, file))).length / 1024

/** The entry chunk is the one `index.html` loads directly. */
const entryName = (): string => {
  const html = readFileSync(join(DIST, 'index.html'), 'utf8')
  const match = /\/static\/assets\/(index-[^"']+\.js)/.exec(html)
  if (!match?.[1]) {
    throw new Error(`index.html does not reference an entry chunk:\n${html}`)
  }
  return match[1]
}

describe('the built bundle', () => {
  beforeAll(() => {
    if (!existsSync(ASSETS)) {
      throw new Error(
        `${ASSETS} does not exist -- run \`npm run build\` before \`npm test\`, ` +
          'which CI does in that order',
      )
    }
  })

  it('keeps the entry chunk within its budget', () => {
    // Measured 73.9 KB: react, react-dom, react-router's component API and the
    // shell. It was 90 with `createBrowserRouter`, whose loaders and fetchers
    // this app does not use -- see app/routes.tsx.
    const BUDGET_KB = 82
    const size = gzippedKb(entryName())
    expect(size).toBeLessThanOrEqual(BUDGET_KB)
  })

  it('keeps every lazy chunk within its budget', () => {
    // Measured: the largest page chunk is 4.4 KB (the proxy form) and the editor
    // is 11.9 -- prism plus four grammars, json, markup, regex and yaml. The page
    // budget is deliberately looser than measured-plus-ten: a page gaining a
    // section is ordinary and should not need this file edited, while a page
    // gaining a *library* is what this is here to catch, and any library clears
    // 8 KB.
    const PAGE_BUDGET_KB = 8
    const EDITOR_BUDGET_KB = 14
    // Measured 37.2: a YAML parser and a JSON Schema validator, for the operators who
    // turn the toggle on. Nobody else fetches it. `js-yaml` would have been 14 KB
    // lighter and brings `argparse`, whose Python-2.0 licence is outside this
    // repository's policy -- see the note in YamlMode.tsx.
    const YAML_BUDGET_KB = 40
    const entry = entryName()

    for (const file of readdirSync(ASSETS).filter((f) => f.endsWith('.js') && f !== entry)) {
      const budget = file.includes('CodeEditor')
        ? EDITOR_BUDGET_KB
        : file.includes('YamlMode')
          ? YAML_BUDGET_KB
          : PAGE_BUDGET_KB
      expect(gzippedKb(file)).toBeLessThanOrEqual(budget)
    }
  })

  it('keeps the stylesheet within its budget', () => {
    // Measured 6.0 KB: 5.1 for the page and 0.9 for the editor's palette, which
    // loads with the editor. Tailwind emits only the utilities the source actually
    // uses, so this grows with the number of distinct classes, not with the app.
    const BUDGET_KB = 7
    const total = readdirSync(ASSETS)
      .filter((file) => file.endsWith('.css'))
      .reduce((sum, file) => sum + gzippedKb(file), 0)
    expect(total).toBeLessThanOrEqual(BUDGET_KB)
  })

  it('keeps the whole payload within its budget', () => {
    // Measured 141 KB across every chunk and the stylesheet, of which 37 is the YAML
    // editor that most visitors never open and 12 the highlighter that only a page
    // with an editor on it fetches. The sum is here so that splitting a chunk in two
    // cannot slip past the per-chunk budgets while making the payload heavier overall.
    const BUDGET_KB = 155
    const total = readdirSync(ASSETS).reduce((sum, file) => sum + gzippedKb(file), 0)
    expect(total).toBeLessThanOrEqual(BUDGET_KB)
  })

  it('ships the highlighter with its colours, and neither in the entry', () => {
    // Two failures in one assertion, because they are the same mistake seen from
    // either side. prism's grammars ship in its package and its palette does not,
    // so an editor built without the theme tokenises perfectly and renders in one
    // colour -- which is how this shipped at first, and looks exactly like no
    // highlighting at all. And the palette belongs in the editor's own stylesheet:
    // in the entry one, every visitor pays for it to list proxies.
    const styles = readdirSync(ASSETS).filter((file) => file.endsWith('.css'))
    const editorStyles = styles.filter((file) => file.includes('CodeEditor'))
    expect(editorStyles).toHaveLength(1)

    const palette = readFileSync(join(ASSETS, editorStyles[0]!), 'utf8')
    expect(palette).toMatch(/\.token\.string/)

    for (const file of styles.filter((name) => !name.includes('CodeEditor'))) {
      expect(readFileSync(join(ASSETS, file), 'utf8')).not.toMatch(/\.token\./)
    }
  })

  it('does not carry the syntax highlighter in the entry chunk', () => {
    // The whole reason the editor is a lazy chunk. A budget alone would not
    // catch this: prism is small enough to hide inside the entry's slack, and
    // then every visitor pays for it to list proxies.
    const entry = readFileSync(join(ASSETS, entryName()), 'utf8')
    expect(entry).not.toMatch(/prism/i)
  })

  it('does not carry the YAML parser or the schema validator in the entry chunk', () => {
    // Same argument, twenty-three kilobytes louder. Matched on strings the two
    // libraries carry in their own messages, since a minifier keeps those and keeps
    // nothing of their names.
    const entry = readFileSync(join(ASSETS, entryName()), 'utf8')
    expect(entry).not.toMatch(/YAMLException/)
    expect(entry).not.toMatch(/additional properties schema/)
  })
})
