// What the API returns besides proxies, and the one error type the whole app
// throws.

/** The six things access is decided for. */
export type Action = 'list' | 'read' | 'create' | 'update' | 'delete' | 'upload'

/** The four a proxy's own `access` block may override. */
export type ProxyAction = 'read' | 'update' | 'delete' | 'upload'

export type CallerView =
  | { kind: 'anonymous' }
  | { kind: 'token'; name: string; group: string }

export interface AccessReport {
  caller: CallerView
  global: Record<Action, boolean>
  /**
   * Absent -- not empty -- when the caller may not `list`. The server withholds
   * it rather than emptying it, because a map keyed by proxy name would be a
   * proxy listing by another route.
   */
  proxies?: Record<string, Record<ProxyAction, boolean>>
}

export interface ProxyStatus {
  name: string
  upstream: string
  resolve: string
  mocks: number
}

export interface Status {
  uptime_seconds: number
  revision: string
  proxies: ProxyStatus[]
}

export interface ReloadReport {
  revision: string
  proxies: number
  /** Sections that changed but need a restart. Absent when empty. */
  unapplied?: string[]
}

export interface TemplateEntry {
  /** The file name. `name` on the wire, which is what the server calls it. */
  name: string
  /** Size in bytes. */
  size: number
}

/** What the page is told by the HTML it was served in. */
export interface RuntimeConfig {
  title: string
  /** The admin API is unauthenticated; the page never asks for a token. */
  public: boolean
  version: string
  authHeader: string
  refreshMs: number
}
