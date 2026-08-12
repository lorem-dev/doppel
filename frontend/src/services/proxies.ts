// The proxy endpoints.

import type { ProxyConfig, ProxyView } from '../types/proxy'
import { request } from './http'

interface ProxyList {
  proxies: ProxyView[]
}

export async function listProxies(): Promise<ProxyView[]> {
  const { proxies } = await request<ProxyList>('/api/v1/proxies')
  return proxies
}

export function readProxy(name: string): Promise<ProxyView> {
  return request<ProxyView>(`/api/v1/proxies/${encodeURIComponent(name)}`)
}

export function createProxy(proxy: ProxyConfig): Promise<ProxyView> {
  return request<ProxyView>('/api/v1/proxies', { method: 'POST', body: { proxy } })
}

/**
 * Replace a proxy, refusing to overwrite a change made since it was read.
 *
 * `revision` is not optional and deliberately so: the server answers 428 without
 * it, and a caller that could omit it would eventually be a caller that does.
 */
export function updateProxy(
  name: string,
  revision: string,
  proxy: ProxyConfig,
): Promise<ProxyView> {
  return request<ProxyView>(`/api/v1/proxies/${encodeURIComponent(name)}`, {
    method: 'PUT',
    body: { revision, proxy },
    ifMatch: revision,
  })
}

export function deleteProxy(name: string, revision: string): Promise<void> {
  return request<void>(`/api/v1/proxies/${encodeURIComponent(name)}`, {
    method: 'DELETE',
    ifMatch: revision,
  })
}
