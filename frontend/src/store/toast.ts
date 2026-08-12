// Short-lived messages: what a write did, and what it refused to do.

import { create } from 'zustand'

export type ToastKind = 'done' | 'failed'

export interface Toast {
  id: number
  kind: ToastKind
  text: string
}

/**
 * Ids come from a counter, not from a clock or a random number.
 *
 * Two toasts raised in the same millisecond would collide on a timestamp, and
 * React would then reuse one row's DOM for the other's text.
 */
let nextId = 1

interface ToastState {
  items: Toast[]
  push: (kind: ToastKind, text: string) => void
  dismiss: (id: number) => void
}

export const useToasts = create<ToastState>((set) => ({
  items: [],
  push: (kind, text) => {
    set((state) => ({ items: [...state.items, { id: nextId++, kind, text }] }))
  },
  dismiss: (id) => {
    set((state) => ({ items: state.items.filter((toast) => toast.id !== id) }))
  },
}))
