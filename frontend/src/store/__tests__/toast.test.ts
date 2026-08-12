import { TOAST_MS, useToasts } from '../toast'

beforeEach(() => {
  jest.useFakeTimers()
  useToasts.setState({ items: [] })
})
afterEach(() => {
  jest.useRealTimers()
})

describe('a toast', () => {
  it('goes away on its own', () => {
    // They only went away on click, so three reloads left three notices stacked in
    // the corner for the rest of the session.
    useToasts.getState().push('done', 'Reloaded')
    expect(useToasts.getState().items).toHaveLength(1)

    jest.advanceTimersByTime(TOAST_MS.done - 1)
    expect(useToasts.getState().items).toHaveLength(1)
    jest.advanceTimersByTime(1)
    expect(useToasts.getState().items).toHaveLength(0)
  })

  it('gives a failure longer than a success', () => {
    // The one an operator may want to copy, and the one that arrives while they are
    // looking somewhere else.
    expect(TOAST_MS.failed).toBeGreaterThan(TOAST_MS.done)

    useToasts.getState().push('failed', 'Refused')
    jest.advanceTimersByTime(TOAST_MS.done)
    expect(useToasts.getState().items).toHaveLength(1)
    jest.advanceTimersByTime(TOAST_MS.failed - TOAST_MS.done)
    expect(useToasts.getState().items).toHaveLength(0)
  })

  it('can still be dismissed by hand before its time', () => {
    useToasts.getState().push('done', 'Reloaded')
    const [only] = useToasts.getState().items
    useToasts.getState().dismiss(only!.id)
    expect(useToasts.getState().items).toHaveLength(0)
  })

  it('keeps several apart, and expires each on its own clock', () => {
    useToasts.getState().push('done', 'first')
    jest.advanceTimersByTime(2000)
    useToasts.getState().push('done', 'second')

    jest.advanceTimersByTime(TOAST_MS.done - 2000)
    expect(useToasts.getState().items.map((toast) => toast.text)).toEqual(['second'])
  })
})
