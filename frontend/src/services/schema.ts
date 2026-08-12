// The configuration schema, from the process serving this page.

import type { JsonSchema } from '../types/schema'
import { request } from './http'

/**
 * `GET /api/v1/schema`.
 *
 * Fetched rather than bundled: it is 30 KB of JSON that every visitor would
 * otherwise pay for, and a bundled copy would describe the version the assets were
 * built from rather than the version that is running.
 */
export function fetchSchema(): Promise<JsonSchema> {
  return request<JsonSchema>('/api/v1/schema')
}
