// What the caller may do, from the server rather than guessed at.

import type { AccessReport } from '../types/api'
import { request } from './http'

export function fetchAccess(): Promise<AccessReport> {
  return request<AccessReport>('/api/v1/access')
}
