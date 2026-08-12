// The configuration shapes the admin API speaks, as TypeScript.
//
// Hand-written rather than generated from `doppel-config.schema.json`. A
// generator would be a build step, a dependency and a drift check of its own;
// what actually goes wrong with a hand-written model is a field nobody added a
// control for, and `schema-drift.test.ts` reads the schema and fails on exactly
// that. Optional here means optional there: a field the server omits is a field
// this must not require.

/** `public`, or the token and group names allowed to act. */
export type Subjects = 'public' | string[]

export interface ProxyAccessConfig {
  read?: Subjects
  update?: Subjects
  delete?: Subjects
  upload?: Subjects
}

export interface LossConfig {
  /** A fraction, not a percentage: 0.5 is half the requests. */
  percentage: number
  status: number
}

export interface LatencyConfig {
  percentage: number
  /** Seconds. */
  min: number
  max: number
}

export type HttpMethod = 'GET' | 'HEAD' | 'POST' | 'PUT' | 'PATCH' | 'DELETE' | 'OPTIONS'

export interface MockRequest {
  method: HttpMethod
  /** A regex matched against the path, unanchored. */
  url: string
  /** Variable name -> request header name. */
  headers?: Record<string, string>
  /** Variable name -> selector. */
  query?: Record<string, string>
  body?: Record<string, string>
}

/**
 * `body`, `json` and `template` are mutually exclusive: exactly one of them is a
 * mock's response source. The form enforces that with a radio rather than
 * hoping, because the server refuses a document with two of them set.
 */
export interface MockResponse {
  status: number
  body?: string
  json?: string
  template?: string
  headers?: Record<string, string>
}

export interface MockProxyOverride {
  replace?: number
  loss?: LossConfig
  latency?: LatencyConfig
}

export interface MockConfig {
  name: string
  request: MockRequest
  response: MockResponse
  proxy?: MockProxyOverride
}

export interface ResolveConfig {
  type: 'default' | 'header'
  header?: string
}

export interface ProxyConfig {
  name: string
  /** Only `http`. `tcp` is not implemented and the server refuses it. */
  type: 'http'
  url: string
  /** Seconds. */
  timeout?: number
  resolve?: ResolveConfig
  access?: ProxyAccessConfig
  headers?: Record<string, string>
  loss?: LossConfig
  latency?: LatencyConfig
  replace?: number
  rewrite_redirects?: boolean
  rewrite_urls?: boolean
  /** Bytes, or a string such as `4Mi`. */
  body_limit?: number | string
  mocks?: MockConfig[]
}

/** A proxy as the API returns it: the document plus the revision to send back. */
export interface ProxyView {
  revision: string
  proxy: ProxyConfig
}
