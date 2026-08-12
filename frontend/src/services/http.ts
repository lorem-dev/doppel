// One place that talks to the API.
//
// Everything above this layer sees `ApiError` and typed results; nothing above it
// sees a `Response`, a status code or the token header's name.

import { ApiError, TRANSPORT_CODE } from '../types/error'
import { runtimeConfig } from './runtimeConfig'
import { loadToken } from './auth'

interface RequestOptions {
  method?: 'GET' | 'POST' | 'PUT' | 'DELETE'
  /** Sent as JSON. */
  body?: unknown
  /** Sent verbatim, for template content. */
  text?: string
  /** The revision a write was built from, sent as `If-Match`. */
  ifMatch?: string
}

/** The server's error envelope. */
interface ErrorBody {
  status?: string
  message?: string
  code?: string
}

/**
 * Call the admin API.
 *
 * Throws `ApiError` for anything that is not a 2xx, including a failure to reach
 * the server at all -- a caller that has to distinguish "no network" from "403"
 * before it can show a message would end up with two error paths and one of them
 * untested.
 */
export async function request<T>(path: string, options: RequestOptions = {}): Promise<T> {
  const { authHeader } = runtimeConfig()
  const headers: Record<string, string> = {}

  const token = loadToken()
  if (token) {
    headers[authHeader] = `Bearer ${token}`
  }
  if (options.body !== undefined) {
    headers['Content-Type'] = 'application/json'
  }
  if (options.ifMatch) {
    // Quoted, because that is what an ETag is. The server compares the value it
    // sent, and it sent it quoted.
    headers['If-Match'] = `"${options.ifMatch}"`
  }

  let response: Response
  try {
    response = await fetch(path, {
      method: options.method ?? 'GET',
      headers,
      body: options.body === undefined ? options.text : JSON.stringify(options.body),
      // The dashboard is served from the same origin it calls, so nothing here
      // needs credentials or a cross-origin mode.
      cache: 'no-store',
    })
  } catch (cause) {
    throw new ApiError(
      0,
      TRANSPORT_CODE,
      `cannot reach the admin API: ${cause instanceof Error ? cause.message : String(cause)}`,
    )
  }

  if (response.status === 204) {
    return undefined as T
  }

  const text = await response.text()

  if (!response.ok) {
    throw await failure(response, text)
  }

  if (!text) {
    return undefined as T
  }
  try {
    return JSON.parse(text) as T
  } catch (cause) {
    throw new ApiError(
      response.status,
      TRANSPORT_CODE,
      `the API returned a body that is not JSON: ${cause instanceof Error ? cause.message : String(cause)}`,
    )
  }
}

/**
 * Turn a failed response into an `ApiError`.
 *
 * A non-envelope body is expected rather than exceptional: a proxy in front of
 * Doppel can answer 502 with HTML, and the operator still needs to be told the
 * status rather than shown a parse error from the error path.
 */
async function failure(response: Response, text: string): Promise<ApiError> {
  let envelope: ErrorBody
  try {
    envelope = JSON.parse(text) as ErrorBody
  } catch {
    envelope = {}
  }

  const message =
    typeof envelope.message === 'string' && envelope.message
      ? envelope.message
      : `the API answered ${response.status} ${response.statusText}`
  const code = typeof envelope.code === 'string' && envelope.code ? envelope.code : TRANSPORT_CODE

  return new ApiError(response.status, code, message)
}
