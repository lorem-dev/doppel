// The form covers every field of a proxy. This is what keeps that true.
//
// A structured form's real failure mode is not a bug in a control -- it is a
// field the configuration format grew that nobody added a control for. Nothing
// fails when that happens: the form saves a document without the field, the
// server accepts it, and the operator quietly loses the setting they had.
//
// So this reads the generated schema -- the same file the Rust drift test holds
// the code to -- and fails when a property is not mentioned anywhere in the form.
// Mentioned is a coarse test, and deliberately so: it cannot prove a control is
// correct, only that the field was not forgotten, which is the failure that
// actually happens.
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

const schema = JSON.parse(
  readFileSync(join(__dirname, '../../../doppel-config.schema.json'), 'utf8'),
) as { $defs: Record<string, { properties?: Record<string, unknown> }> }

/** The form's source, concatenated. */
const source = ['../pages/ProxyFormPage.tsx', '../components/MockEditor.tsx']
  .map((file) => readFileSync(join(__dirname, file), 'utf8'))
  .join('\n')

/** The TypeScript model the form edits. */
const model = readFileSync(join(__dirname, '../types/proxy.ts'), 'utf8')

const propertiesOf = (name: string): string[] => {
  const definition = schema.$defs[name]
  if (!definition?.properties) {
    throw new Error(`${name} is not in the schema, or has no properties`)
  }
  return Object.keys(definition.properties)
}

describe.each([
  ['ProxyConfig'],
  ['MockConfig'],
  ['MockRequest'],
  ['MockResponse'],
  ['ResolveConfig'],
  ['LossConfig'],
  ['LatencyConfig'],
  ['ProxyAccessConfig'],
])('%s', (definition) => {
  it('has every property in the form model', () => {
    for (const property of propertiesOf(definition)) {
      expect(model).toContain(property)
    }
  })

  it('has every property mentioned by the form', () => {
    for (const property of propertiesOf(definition)) {
      expect(source).toContain(property)
    }
  })
})

describe('the schema itself', () => {
  it('is the one the server generates', () => {
    // A guard on the two paths above: if the schema moved, both loops would pass
    // against an empty file and this suite would report success while checking
    // nothing.
    expect(Object.keys(schema.$defs).length).toBeGreaterThan(20)
    expect(propertiesOf('ProxyConfig')).toContain('url')
  })
})
