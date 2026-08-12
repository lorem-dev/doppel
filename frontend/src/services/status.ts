// The process endpoints.

import type { ReloadReport, Status } from '../types/api'
import { request } from './http'

export function fetchStatus(): Promise<Status> {
  return request<Status>('/status')
}

export function reload(): Promise<ReloadReport> {
  return request<ReloadReport>('/api/v1/config/reload', { method: 'POST' })
}
