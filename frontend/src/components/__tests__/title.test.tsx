// How a deployment's own name is split into tones.
//
// The wordmark is `Doppel` + `ganger`, and a title gets the same treatment: the
// first word in the strong tone, everything after it in the light one. What counts
// as a word is the interesting part, and it is all in `titleSegments`.

import { titleSegments } from '../Title'

describe('titleSegments', () => {
  it('splits on the three things a name uses for a boundary', () => {
    expect(titleSegments('Billing API')).toEqual(['Billing', ' API'])
    expect(titleSegments('billing_api')).toEqual(['billing', '_api'])
    expect(titleSegments('billingApi')).toEqual(['billing', 'Api'])
    // A hyphen too: `billing-api` is the same name written the way a hostname is.
    expect(titleSegments('billing-api')).toEqual(['billing', '-api'])
  })

  it('keeps every character, so the title reads as it was typed', () => {
    for (const title of ['Billing API', 'billing_api', 'billingApi', 'a b  c', 'Doppel (e2e)']) {
      expect(titleSegments(title).join('')).toBe(title)
    }
  })

  it('leaves a one-word title as one segment', () => {
    // Which gets the strong tone, so a single word is not drawn in the accent
    // colour for no reason.
    expect(titleSegments('billing')).toEqual(['billing'])
    expect(titleSegments('API')).toEqual(['API'])
  })

  it('does not split inside an acronym or a number', () => {
    // `API2` is one word: a case change is a boundary between two letters, and a
    // run of capitals is not a run of words.
    expect(titleSegments('APIGateway')).toEqual(['APIGateway'])
    expect(titleSegments('proxy2Api')).toEqual(['proxy2', 'Api'])
  })

  it('has nothing to say about an empty title', () => {
    expect(titleSegments('')).toEqual([])
  })
})
