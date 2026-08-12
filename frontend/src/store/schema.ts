// The configuration schema, fetched once and read by every form.
//
// A store rather than a fetch per page: the document is the same for the life of
// the process, four pages want it, and a page that fetched it for itself would
// refetch 30 KB on every navigation.

import { useCallback } from 'react'
import { create } from 'zustand'

import type { JsonSchema } from '../types/schema'
import type { Rule } from '../schema/rules'
import { ruleAt, valueRuleAt } from '../schema/rules'
import { fetchSchema } from '../services/schema'

interface SchemaState {
  schema?: JsonSchema
  /**
   * The fetch failed.
   *
   * Not shown anywhere and not meant to be: the schema tightens the form, it does
   * not enable it. A deployment behind something that eats the endpoint gets a form
   * whose bounds are checked by the server on save, which is where they were
   * checked before this existed.
   */
  error?: string
  load: () => Promise<void>
}

export const useSchema = create<SchemaState>((set, get) => ({
  load: async () => {
    if (get().schema) {
      return
    }
    try {
      set({ schema: await fetchSchema(), error: undefined })
    } catch (caught) {
      set({ error: caught instanceof Error ? caught.message : String(caught) })
    }
  },
}))

/**
 * The rule for a field of a proxy document, by path.
 *
 * Subscribes to the schema, so a form rendered before the fetch finished re-renders
 * with its bounds when it arrives -- the same mistake `useMay` documents, where
 * selecting a stable function left every control frozen in its initial state.
 */
export function useRule(): (path: string) => Rule | undefined {
  const schema = useSchema((state) => state.schema)
  return useCallback((path: string) => ruleAt(schema, path), [schema])
}

/** The rule for the *values* of a map field, by path. */
export function useValueRule(): (path: string) => Rule | undefined {
  const schema = useSchema((state) => state.schema)
  return useCallback((path: string) => valueRuleAt(schema, path), [schema])
}
