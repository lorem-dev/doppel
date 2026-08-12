// The proxy list, refetched on a timer.

import { create } from 'zustand'

import type { ProxyView } from '../types/proxy'
import { ApiError } from '../types/error'
import { deleteProxy, listProxies } from '../services/proxies'

interface ProxiesState {
  items: ProxyView[]
  error?: ApiError
  /** True only for the first load, so a refetch does not blank the table. */
  loading: boolean
  load: () => Promise<void>
  remove: (name: string, revision: string) => Promise<void>
}

export const useProxies = create<ProxiesState>((set, get) => ({
  items: [],
  loading: true,

  load: async () => {
    try {
      set({ items: await listProxies(), error: undefined, loading: false })
    } catch (caught) {
      // The previous list is kept. A refetch that fails once -- a reload
      // restarting the listener, say -- should not empty a table the operator is
      // reading, and the error is shown alongside it.
      set({ error: caught as ApiError, loading: false })
    }
  },

  remove: async (name, revision) => {
    await deleteProxy(name, revision)
    // Refetched rather than spliced out locally: the delete may have raced
    // another change, and the list is the server's to describe.
    await get().load()
  },
}))
