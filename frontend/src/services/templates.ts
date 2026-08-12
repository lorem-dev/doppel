// A proxy's template files.
//
// There is no file upload: content is typed into the editor and sent as the
// request body, which is what the endpoint has always taken. A file picker would
// add a second way in for no gain -- the files are small templates, not
// artifacts.

import type { TemplateEntry } from '../types/api'
import { request } from './http'

interface TemplateList {
  templates: TemplateEntry[]
}

export async function listTemplates(proxy: string): Promise<TemplateEntry[]> {
  const { templates } = await request<TemplateList>(
    `/api/v1/proxies/${encodeURIComponent(proxy)}/templates`,
  )
  return templates
}

export function putTemplate(proxy: string, file: string, content: string): Promise<void> {
  return request<void>(
    `/api/v1/proxies/${encodeURIComponent(proxy)}/templates/${encodeURIComponent(file)}`,
    { method: 'POST', text: content },
  )
}

export function deleteTemplate(proxy: string, file: string): Promise<void> {
  return request<void>(
    `/api/v1/proxies/${encodeURIComponent(proxy)}/templates/${encodeURIComponent(file)}`,
    { method: 'DELETE' },
  )
}
