// The field rules, read out of the real schema.
//
// Against the checked-in `doppel-config.schema.json` rather than a fixture, on
// purpose: a fixture would be a third copy of the bounds and would keep passing
// after the types changed. The Rust drift test holds that file to what the code
// generates, and `GET /api/v1/schema` serves the same document, so a rule proved
// here is a rule the server enforces.
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

import type { JsonSchema } from '../../types/schema'
import { complain, numericAttrs, ruleAt, summarize, valueRuleAt } from '../rules'

const schema = JSON.parse(
  readFileSync(join(__dirname, '../../../../doppel-config.schema.json'), 'utf8'),
) as JsonSchema

describe('reading a rule out of the schema', () => {
  it('finds a pattern and a length on a name', () => {
    const rule = ruleAt(schema, 'name')
    expect(rule?.pattern).toBe('^[A-Za-z0-9_-]{2,32}$')
    expect(rule?.maxLength).toBe(32)
    expect(rule?.required).toBe(true)
  })

  it('follows an optional field past the null branch to its bounds', () => {
    // `timeout` is an `Option<TimeoutSeconds>`, which arrives as
    // `oneOf: [{type: null}, {$ref}]` -- so a reader that stopped at the field
    // would find no bounds at all.
    const rule = ruleAt(schema, 'timeout')
    expect(rule?.minimum).toBe(1)
    expect(rule?.maximum).toBe(3600)
    expect(rule?.type).toBe('integer')
    expect(rule?.required).toBe(false)
  })

  it('walks into a nested object and into a list', () => {
    expect(ruleAt(schema, 'loss.percentage')?.maximum).toBe(1)
    expect(ruleAt(schema, 'latency.max')?.maximum).toBe(300)
    expect(ruleAt(schema, 'mocks[].name')?.pattern).toBe('^[A-Za-z0-9_-]{2,64}$')
    expect(ruleAt(schema, 'mocks[].response.status')?.minimum).toBe(100)
    expect(ruleAt(schema, 'mocks[].request.method')?.enum).toContain('PATCH')
  })

  it('keeps both spellings of a field that accepts two', () => {
    // `body_limit` is an integer or a string like `4Mi`.
    const rule = ruleAt(schema, 'body_limit')
    expect(rule?.either).toHaveLength(2)
    expect(rule?.either?.[0]?.maximum).toBe(1073741824)
  })

  it('reads the value schema of a map', () => {
    // The interesting half of a variable map: the selector has a pattern.
    expect(valueRuleAt(schema, 'mocks[].request.query')?.pattern).toBe('^\\.[^.]+(\\.[^.]+)*$')
  })

  it('has nothing to say about a path it does not describe', () => {
    expect(ruleAt(schema, 'nonesuch')).toBeUndefined()
    expect(ruleAt(schema, 'mocks[].nonesuch')).toBeUndefined()
    expect(ruleAt(undefined, 'name')).toBeUndefined()
  })
})

describe('complaining about a value', () => {
  const nameRule = ruleAt(schema, 'name')
  const timeoutRule = ruleAt(schema, 'timeout')

  it('says nothing about an empty box', () => {
    // Empty is how the form spells an absent optional field, and an error under a
    // field nobody has typed in yet is an accusation about the future.
    expect(complain(nameRule, '')).toBeUndefined()
    expect(complain(timeoutRule, undefined)).toBeUndefined()
  })

  it('says nothing about a legal value', () => {
    expect(complain(nameRule, 'billing-api')).toBeUndefined()
    expect(complain(timeoutRule, 30)).toBeUndefined()
    expect(complain(ruleAt(schema, 'replace'), 0.25)).toBeUndefined()
  })

  it('explains a name that breaks the pattern in the schema words', () => {
    // The message is the type's own description, so the page and the server say the
    // same thing about the same rule.
    expect(complain(nameRule, 'has space')).toBe(
      'Letters, digits, - and _, between 2 and 32 characters.',
    )
    expect(complain(nameRule, 'a')).toBe('at least 2 characters')
    expect(complain(nameRule, 'x'.repeat(33))).toBe('at most 32 characters; this is 33')
  })

  it('states a range as a range', () => {
    expect(complain(timeoutRule, 0)).toBe('must be between 1 and 3600')
    expect(complain(timeoutRule, 5000)).toBe('must be between 1 and 3600')
    expect(complain(ruleAt(schema, 'loss.percentage'), 45)).toBe('must be between 0 and 1')
    expect(complain(ruleAt(schema, 'mocks[].response.status'), 99)).toBe(
      'must be between 100 and 599',
    )
  })

  it('refuses a fraction where the type is whole', () => {
    expect(complain(timeoutRule, 1.5)).toBe('must be a whole number')
  })

  it('accepts either spelling of a two-shaped field, and refuses neither', () => {
    const rule = ruleAt(schema, 'body_limit')
    expect(complain(rule, '4Mi')).toBeUndefined()
    expect(complain(rule, '1048576')).toBeUndefined()
    // Digits are the integer spelling, so the integer branch's bound applies. The
    // string branch carries no bound -- judging by it would let this through and
    // leave the refusal to the server.
    expect(complain(rule, '9999999999')).toBe('must be between 1 and 1073741824')
    expect(complain(rule, '0')).toBe('must be between 1 and 1073741824')
  })

  it('checks a selector as it is typed', () => {
    const rule = valueRuleAt(schema, 'mocks[].request.body')
    expect(complain(rule, '.content.items')).toBeUndefined()
    expect(complain(rule, 'content.items')).toBe(
      'A selector addressing object keys: a leading dot, then dot-separated field names, as in .content.items.',
    )
  })
})

describe('the attributes a number input can carry', () => {
  it('takes the range from the schema, and a step only for whole numbers', () => {
    expect(numericAttrs(ruleAt(schema, 'timeout'))).toEqual({ min: 1, max: 3600, step: 1 })
    expect(numericAttrs(ruleAt(schema, 'latency.min'))).toEqual({ min: 0, max: 300 })
  })

  it('has nothing to add to a text field', () => {
    expect(numericAttrs(ruleAt(schema, 'name'))).toEqual({})
    expect(numericAttrs(undefined)).toEqual({})
  })
})

describe('summarizing a description', () => {
  it('keeps the first sentence and drops the markup', () => {
    expect(summarize('One thing. And then another thing entirely.')).toBe('One thing.')
    expect(summarize('A `code` word.')).toBe('A code word.')
    expect(summarize(undefined)).toBeUndefined()
  })
})
