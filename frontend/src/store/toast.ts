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

/**
 * How long a message stays.
 *
 * Long enough to read a revision out of, short enough that three reloads do not
 * leave three notices stacked in the corner for the rest of the session -- which
 * is what happened when they only went away on click.
 *
 * A failure stays longer: it is the one an operator may want to copy, and it is
 * the one that arrives while they are looking elsewhere.
 */
export const TOAST_MS: Record<ToastKind, number> = { done: 4000, failed: 10_000 }

interface ToastState {
  items: Toast[]
  push: (kind: ToastKind, text: string) => void
  dismiss: (id: number) => void
}

export const useToasts = create<ToastState>((set, get) => ({
  items: [],
  push: (kind, text) => {
    const id = nextId++
    set((state) => ({ items: [...state.items, { id, kind, text }] }))
    // Timed here rather than in the component that renders them: a toast raised by
    // a page that then navigates away would otherwise never be cleaned up, because
    // the timer would have gone with the unmounted component.
    setTimeout(() => get().dismiss(id), TOAST_MS[kind])
  },
  dismiss: (id) => {
    set((state) => ({ items: state.items.filter((toast) => toast.id !== id) }))
  },
}))
