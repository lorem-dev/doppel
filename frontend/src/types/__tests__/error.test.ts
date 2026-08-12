import { ApiError, complaintAbout } from '../error'

/**
 * The two message shapes the server actually produces, quoted from live
 * responses rather than invented.
 *
 * The first version of this parser split on newlines and matched neither, so
 * every field-level error silently fell back to the banner and the tests passed
 * because the banner contains the same words. These fixtures are the fix.
 */
const VALIDATION = new ApiError(
  400,
  'CONFIG_INVALID',
  'configuration is invalid: proxies[0].url: must be absolute; ' +
    'proxies[1].name: duplicate proxy name `alpha`',
)

const PARSE = new ApiError(
  400,
  'CONFIG_INVALID',
  'request body is not a valid proxy document: url must be absolute: ' +
    'relative URL without a base at line 1 column 55',
)

describe('a validation refusal', () => {
  it('answers for a field named in a path', () => {
    expect(complaintAbout(VALIDATION, 'url')).toBe('must be absolute')
    expect(complaintAbout(VALIDATION, 'name')).toBe('duplicate proxy name `alpha`')
  })

  it('says nothing about a field it does not mention', () => {
    expect(complaintAbout(VALIDATION, 'timeout')).toBeUndefined()
    expect(complaintAbout(VALIDATION, 'replace')).toBeUndefined()
  })

  it('does not let one field answer for another that starts the same way', () => {
    // `body` and `body_limit` are both real fields, and a prefix match would put
    // the wrong complaint under the wrong control.
    const error = new ApiError(400, 'CONFIG_INVALID', 'configuration is invalid: proxies[0].body_limit: must not be zero')
    expect(complaintAbout(error, 'body_limit')).toBe('must not be zero')
    expect(complaintAbout(error, 'body')).toBeUndefined()
  })

  it('distinguishes a mocks path from the proxy field of the same name', () => {
    const error = new ApiError(
      400,
      'CONFIG_INVALID',
      'configuration is invalid: proxies[0].mocks[1].name: duplicate mock name',
    )
    expect(complaintAbout(error, 'mocks[1].name')).toBe('duplicate mock name')
    expect(complaintAbout(error, 'mocks[0].name')).toBeUndefined()
  })
})

describe('a parse refusal', () => {
  it('answers for the field serde named', () => {
    // serde puts the field first and its own detail after, with no path: the
    // whole remainder is the message.
    expect(complaintAbout(PARSE, 'url')).toBe(
      'must be absolute: relative URL without a base at line 1 column 55',
    )
  })

  it('does not invent a field out of ordinary prose', () => {
    // The failure mode of reading paths out of a message instead of asking about
    // a field: "no" is the first word of this one.
    const error = new ApiError(404, 'PROXY_NOT_RESOLVED', 'no proxy matched the request')
    expect(complaintAbout(error, 'no')).toBeUndefined()
    expect(complaintAbout(error, 'url')).toBeUndefined()
  })
})

describe('the error itself', () => {
  it('classifies what a caller has to branch on', () => {
    expect(new ApiError(401, 'UNAUTHORIZED', 'x').isAuth).toBe(true)
    expect(new ApiError(403, 'FORBIDDEN', 'x').isAuth).toBe(true)
    expect(new ApiError(409, 'CONFLICT', 'x').isStale).toBe(true)
    expect(new ApiError(428, 'REVISION_REQUIRED', 'x').isStale).toBe(true)
    expect(new ApiError(500, 'STORE_ERROR', 'x').isAuth).toBe(false)
    expect(new ApiError(500, 'STORE_ERROR', 'x').isStale).toBe(false)
  })
})
