/**
 * Every failure the app handles, in one shape.
 *
 * The server's error envelope is `{status, message, code}`, and a transport
 * failure has none of those -- so it is given a synthetic code rather than a
 * different type. One type means a component never has to ask which kind of
 * failure it is holding before it can show the message.
 */
export class ApiError extends Error {
  readonly status: number
  readonly code: string

  constructor(status: number, code: string, message: string) {
    super(message)
    this.name = 'ApiError'
    this.status = status
    this.code = code
  }

  /** Whether this is the server asking for a token, or refusing the one given. */
  get isAuth(): boolean {
    return this.status === 401 || this.status === 403
  }

  /** Whether the caller's revision was stale. 428 means none was sent. */
  get isStale(): boolean {
    return this.status === 409 || this.status === 428
  }
}

/** The code used when the request never reached a server. */
export const TRANSPORT_CODE = 'TRANSPORT'

/**
 * The two prefixes the server puts in front of a refusal.
 *
 * One from the validation rules, one from serde when the document does not parse
 * at all. Stripped before anything is read out of the message, because the words
 * before the colon are prose rather than a path.
 */
const PREFIXES = [
  'configuration is invalid:',
  'request body is not a valid proxy document:',
]

/**
 * The server's complaint about one field, if it made one.
 *
 * The form asks about a field it knows rather than a parser inventing paths from
 * the message. That is not fastidiousness: the two shapes a refusal arrives in
 * are
 *
 *   configuration is invalid: proxies[0].url: must be absolute; proxies[1].name: ...
 *   request body is not a valid proxy document: url must be absolute: relative ...
 *
 * and a parser reading paths out of the second one would take `no` for a field
 * name out of "no proxy matched the request". Asking about `url` cannot do that.
 *
 * `field` may be a plain name (`url`) or a path within the document
 * (`mocks[1].name`); a segment matches when its path ends there, so the proxy's
 * own `name` does not answer for a mock's.
 *
 * A message carrying neither prefix is not searched at all. Those are the ones
 * that are prose about the request rather than a list of field complaints -- "no
 * proxy matched the request" would otherwise answer for a field called `no` -- and
 * the banner is where they belong.
 */
export function complaintAbout(error: ApiError, field: string): string | undefined {
  const prefix = PREFIXES.find((candidate) => error.message.startsWith(candidate))
  if (!prefix) {
    return undefined
  }
  const text = error.message.slice(prefix.length)

  for (const segment of text.split(';')) {
    const complaint = segment.trim()
    if (!complaint.startsWith(field)) {
      // Not this field, unless the path leads to it: `proxies[0].url` ends in the
      // `url` the form knows.
      const at = complaint.indexOf(field)
      const before = at > 0 ? complaint[at - 1] : undefined
      if (at <= 0 || (before !== '.' && before !== ' ')) {
        continue
      }
      const rest = tail(complaint.slice(at + field.length))
      if (rest) {
        return rest
      }
      continue
    }
    const rest = tail(complaint.slice(field.length))
    if (rest) {
      return rest
    }
  }
  return undefined
}

/**
 * What follows the field name, once the punctuation between them is gone.
 *
 * `undefined` when the name was only a prefix of a longer one -- `body` must not
 * answer for `body_limit` -- or when nothing follows it at all.
 */
function tail(rest: string): string | undefined {
  if (!rest.startsWith(':') && !rest.startsWith(' ')) {
    return undefined
  }
  const message = rest.replace(/^[:\s]+/, '').trim()
  return message || undefined
}
