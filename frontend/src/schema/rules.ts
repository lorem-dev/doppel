// What a field is allowed to hold, read out of the schema the server serves.
//
// The point of this file is that there is no second copy of the rules. `ProxyName`
// says "letters, digits, - and _, 2 to 32" in one place -- a Rust newtype -- and
// that reaches the page as a `pattern` and a `maxLength` through
// `GET /api/v1/schema`. A rule added there shows up here without anyone editing
// here, and a rule loosened there cannot leave a stricter copy behind.
//
// The server still decides. These bounds are the subset a single field can be
// judged by on its own; anything that needs two fields -- `min <= max`, a template
// a mock must declare, a name nothing else uses -- is refused on save and reported
// against the field it is about.

import type { JsonSchema } from '../types/schema'

/** The bounds on one field, flattened out of the schema. */
export interface Rule {
  type?: string
  description?: string
  examples?: unknown[]
  pattern?: string
  minLength?: number
  maxLength?: number
  minimum?: number
  maximum?: number
  enum?: unknown[]
  /** The field's parent lists it as required. */
  required?: boolean
  /**
   * The alternatives, when a field accepts more than one shape.
   *
   * `body_limit` is `1048576` or `"1Mi"`, and a value is legal if it satisfies
   * either -- which is what `oneOf` means and what `complain` implements.
   */
  either?: Rule[]
}

/** Follow a `$ref` to the definition it names. */
function deref(node: JsonSchema, root: JsonSchema): JsonSchema {
  const ref = node.$ref
  if (!ref) {
    return node
  }
  const name = ref.replace('#/$defs/', '')
  return root.$defs?.[name] ?? node
}

/**
 * The schema of a value, with the wrappers taken off.
 *
 * `Option<T>` arrives as `oneOf: [{type: null}, {$ref}]`, so an optional field's
 * bounds are one level below where a reader looks for them. The null branch is
 * dropped -- absent is always legal here, and a form spells absent as an empty box
 * -- and a single remaining branch is followed. Two real branches are kept, in
 * `either`.
 */
function unwrap(node: JsonSchema, root: JsonSchema): JsonSchema {
  const resolved = deref(node, root)
  if (!resolved.oneOf) {
    return resolved
  }
  const real = resolved.oneOf.filter((branch) => branch.type !== 'null')
  if (real.length === 1 && real[0]) {
    // The description usually sits on the field or the wrapper rather than on the
    // branch, so it is carried down.
    return { description: resolved.description, ...deref(real[0], root) }
  }
  return resolved
}

/** Turn a resolved node into a rule, resolving a two-branch union as well. */
function ruleOf(node: JsonSchema, root: JsonSchema, required: boolean): Rule {
  // Named one by one rather than spread: what a rule holds is a short, closed list,
  // and spreading the node would quietly carry `properties` and `$defs` into every
  // rule for the sake of three fewer lines.
  const rule: Rule = {
    // A list of types is legal JSON Schema and nothing here emits one, so the first
    // is taken rather than a union being invented for a case that does not arise.
    type: Array.isArray(node.type) ? node.type[0] : node.type,
    description: node.description,
    examples: node.examples,
    pattern: node.pattern,
    minLength: node.minLength,
    maxLength: node.maxLength,
    minimum: node.minimum,
    maximum: node.maximum,
    enum: node.enum,
    required,
  }
  if (node.oneOf) {
    const real = node.oneOf.filter((branch) => branch.type !== 'null')
    if (real.length > 1) {
      rule.either = real.map((branch) => ruleOf(deref(branch, root), root, required))
    }
  }
  return rule
}

/**
 * The rule for a field of a proxy document, by path.
 *
 * `name`, `loss.percentage`, `mocks[].request.url`: the path the form uses to talk
 * about a field, where `[]` is a list's element. Undefined when the schema has not
 * arrived yet or does not describe that path -- a caller then simply has nothing to
 * check, which is the state the page starts in.
 */
export function ruleAt(root: JsonSchema | undefined, path: string): Rule | undefined {
  if (!root) {
    return undefined
  }
  let node = unwrap({ $ref: '#/$defs/ProxyConfig' }, root)
  let required = true

  for (const segment of path.split('.')) {
    const list = segment.endsWith('[]')
    const key = list ? segment.slice(0, -2) : segment

    const property = node.properties?.[key]
    if (!property) {
      return undefined
    }
    required = node.required?.includes(key) ?? false
    node = unwrap(property, root)

    if (list) {
      if (!node.items) {
        return undefined
      }
      node = unwrap(node.items, root)
    }
  }

  return ruleOf(node, root, required)
}

/**
 * The rule for the values of a map field, such as a header or a variable map.
 *
 * The values are the interesting half: a variable's selector has a pattern, and a
 * key is a name the server checks. `additionalProperties` is where the value schema
 * lives.
 */
export function valueRuleAt(root: JsonSchema | undefined, path: string): Rule | undefined {
  if (!root) {
    return undefined
  }
  const parent = mapNodeAt(root, path)
  const values = parent?.additionalProperties
  if (!values || typeof values === 'boolean') {
    return undefined
  }
  return ruleOf(unwrap(values, root), root, false)
}

/** The map field itself, for `valueRuleAt`. */
function mapNodeAt(root: JsonSchema, path: string): JsonSchema | undefined {
  let node: JsonSchema = unwrap({ $ref: '#/$defs/ProxyConfig' }, root)
  for (const segment of path.split('.')) {
    const list = segment.endsWith('[]')
    const key = list ? segment.slice(0, -2) : segment
    const property = node.properties?.[key]
    if (!property) {
      return undefined
    }
    node = unwrap(property, root)
    if (list) {
      if (!node.items) {
        return undefined
      }
      node = unwrap(node.items, root)
    }
  }
  return node
}

/**
 * The first sentence of a schema description, as a sentence a control can show.
 *
 * The descriptions come from Rust doc comments, so they are Markdown and some run
 * to a paragraph. A field's error line has room for one sentence, and the first one
 * is the one that states the rule.
 */
export function summarize(description: string | undefined): string | undefined {
  if (!description) {
    return undefined
  }
  const plain = description.replace(/`/g, '')
  const stop = plain.indexOf('. ')
  const first = stop === -1 ? plain : plain.slice(0, stop + 1)
  return first.split('\n')[0]?.trim()
}

/**
 * What is wrong with this value, in one sentence, or nothing.
 *
 * Empty is never a complaint. An empty box is how the form spells an absent
 * optional field, and a required one that is empty is why the save button is
 * disabled -- an error under a field nobody has typed in yet is an accusation about
 * the future.
 */
export function complain(rule: Rule | undefined, value: unknown): string | undefined {
  if (!rule || value === undefined || value === null || value === '') {
    return undefined
  }

  // A union is legal if a branch accepts it -- but only the branches that describe
  // the way this value is spelled. `body_limit` is `1048576` or `"4Mi"`, and both
  // arrive from the form as text: judging `9999999999` by the string branch would
  // find no bound at all and let a value through that the server refuses, because
  // the bound on the number lives on the integer branch. Digits are the integer
  // spelling; anything else is the other one.
  if (rule.either) {
    const digits = /^-?\d+(\.\d+)?$/.test(String(value).trim())
    const numbers = rule.either.filter(
      (branch) => branch.type === 'integer' || branch.type === 'number',
    )
    const rest = rule.either.filter((branch) => !numbers.includes(branch))
    const branches = digits ? (numbers.length ? numbers : rest) : rest.length ? rest : numbers

    const complaints = branches.map((branch) => complain(branch, value))
    return complaints.every(Boolean) ? complaints[0] : undefined
  }

  if (typeof value === 'number') {
    return numeric(rule, value)
  }

  const text = String(value)

  if (rule.enum && !rule.enum.includes(text)) {
    return `must be one of ${rule.enum.join(', ')}`
  }
  if (rule.maxLength !== undefined && text.length > rule.maxLength) {
    return `at most ${rule.maxLength} characters; this is ${text.length}`
  }
  if (rule.minLength !== undefined && text.length < rule.minLength) {
    return `at least ${rule.minLength} characters`
  }
  if (rule.pattern && !new RegExp(rule.pattern).test(text)) {
    // The description states the rule the pattern encodes -- "letters, digits, -
    // and _, between 2 and 32 characters" -- and is a better sentence than the
    // pattern is. The pattern is the fallback for a type whose description says
    // something else.
    return summarize(rule.description) ?? `must match ${rule.pattern}`
  }
  // A number typed into a text box still has bounds to respect: `body_limit` is a
  // string field whose integer spelling has a maximum.
  if (rule.type === 'integer' || rule.type === 'number') {
    const parsed = Number(text)
    return Number.isFinite(parsed) ? numeric(rule, parsed) : undefined
  }
  return undefined
}

function numeric(rule: Rule, value: number): string | undefined {
  const { minimum, maximum } = rule
  const low = minimum !== undefined && value < minimum
  const high = maximum !== undefined && value > maximum
  if (!low && !high) {
    return rule.type === 'integer' && !Number.isInteger(value)
      ? 'must be a whole number'
      : undefined
  }
  if (minimum !== undefined && maximum !== undefined) {
    return `must be between ${minimum} and ${maximum}`
  }
  return low ? `must be at least ${minimum}` : `must be at most ${maximum}`
}

/**
 * The bounds a number input can enforce itself, as attributes.
 *
 * The browser's own spinners and keyboard then stay inside the range, which is a
 * cheaper way to say "1 to 3600" than a message after the fact -- and the message
 * is still there for a pasted value.
 */
export function numericAttrs(rule: Rule | undefined): {
  min?: number
  max?: number
  step?: number
} {
  if (!rule || (rule.type !== 'integer' && rule.type !== 'number')) {
    return {}
  }
  return {
    min: rule.minimum,
    max: rule.maximum,
    // Only for integers. A fractional step is a decision about how finely this
    // particular value is worth nudging, which the schema does not hold and the
    // form does.
    ...(rule.type === 'integer' ? { step: 1 } : {}),
  }
}
