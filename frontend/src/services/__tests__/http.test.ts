import { ApiError, TRANSPORT_CODE } from '../../types/error'
import { request } from '../http'
import { saveToken } from '../auth'
import { forgetRuntimeConfig } from '../runtimeConfig'

/** The configuration the page would have been served with. */
const injectConfig = (authHeader = 'X-Proxy-Authorization'): void => {
  document.head.innerHTML = `<script type="application/json" id="doppel-config">${JSON.stringify(
    { title: 'Doppel', public: false, version: '0.4.1', authHeader, refreshMs: 60000 },
  )}</script>`
  forgetRuntimeConfig()
}

/** The last call's init, so header assertions do not need a mock library. */
let lastInit: RequestInit | undefined

/**
 * Answer the next call with this status and body.
 *
 * A hand-built object rather than a real `Response`: jsdom provides no fetch, so
 * `Response` is not a global here. This carries exactly the four members `http.ts`
 * reads, which also means a fifth one appearing would fail loudly here rather
 * than pass against a fake that quietly has everything.
 */
const respond = (status: number, body: string, statusText = ''): void => {
  global.fetch = jest.fn((_url: unknown, init?: RequestInit) => {
    lastInit = init
    return Promise.resolve({
      status,
      statusText,
      ok: status >= 200 && status < 300,
      text: () => Promise.resolve(body),
    } as unknown as Response)
  }) as unknown as typeof fetch
}

beforeEach(() => {
  localStorage.clear()
  sessionStorage.clear()
  lastInit = undefined
  injectConfig()
})

describe('a successful call', () => {
  it('parses the body', async () => {
    respond(200, '{"proxies":[]}')
    await expect(request<{ proxies: unknown[] }>('/api/v1/proxies')).resolves.toEqual({
      proxies: [],
    })
  })

  it('accepts 204 with no body', async () => {
    respond(204, '')
    await expect(request<void>('/api/v1/proxies/alpha/templates/x.json.j2')).resolves
      .toBeUndefined()
  })
})

describe('the token header', () => {
  it('is absent when no token is held', async () => {
    respond(200, '{}')
    await request('/api/v1/access')
    expect((lastInit?.headers as Record<string, string>)['X-Proxy-Authorization']).toBeUndefined()
  })

  it('uses the name the server configured, not the default', async () => {
    // `admin.auth.header` is configurable, so a hard-coded name would send the
    // token where nothing reads it and the page would look unauthenticated.
    injectConfig('X-Admin-Token')
    saveToken('root-token')
    respond(200, '{}')
    await request('/api/v1/access')

    const headers = lastInit?.headers as Record<string, string>
    expect(headers['X-Admin-Token']).toBe('Bearer root-token')
    expect(headers['X-Proxy-Authorization']).toBeUndefined()
  })
})

describe('a failed call', () => {
  it('becomes an ApiError carrying the envelope', async () => {
    respond(403, '{"status":"error","message":"token `reader` may not `create`","code":"FORBIDDEN"}')

    const error = await request('/api/v1/proxies', { method: 'POST', body: {} }).catch(
      (caught: unknown) => caught,
    )
    expect(error).toBeInstanceOf(ApiError)
    const api = error as ApiError
    expect(api.status).toBe(403)
    expect(api.code).toBe('FORBIDDEN')
    expect(api.message).toContain('may not `create`')
    expect(api.isAuth).toBe(true)
  })

  it('survives a body that is not the envelope', async () => {
    // A proxy in front of Doppel answers 502 with HTML. The operator still needs
    // the status, not a parse error thrown from the error path.
    respond(502, '<html><body>Bad Gateway</body></html>', 'Bad Gateway')

    const error = (await request('/api/v1/proxies').catch((caught: unknown) => caught)) as ApiError
    expect(error.status).toBe(502)
    expect(error.code).toBe(TRANSPORT_CODE)
    expect(error.message).toContain('502')
  })

  it('turns an unreachable server into an ApiError too', async () => {
    global.fetch = jest.fn(() => Promise.reject(new Error('Failed to fetch'))) as unknown as typeof fetch

    const error = (await request('/api/v1/proxies').catch((caught: unknown) => caught)) as ApiError
    expect(error).toBeInstanceOf(ApiError)
    expect(error.status).toBe(0)
    expect(error.code).toBe(TRANSPORT_CODE)
    expect(error.message).toContain('cannot reach')
  })

  it('reports a stale revision distinctly from an auth failure', async () => {
    respond(409, '{"message":"revision moved","code":"REVISION_MISMATCH"}')
    const error = (await request('/api/v1/proxies/alpha', { method: 'PUT' }).catch(
      (caught: unknown) => caught,
    )) as ApiError
    expect(error.isStale).toBe(true)
    expect(error.isAuth).toBe(false)
  })
})

describe('a write', () => {
  it('sends the revision as a quoted If-Match', async () => {
    // Quoted because that is what an ETag is, and the server compares the value
    // it sent -- which it sent quoted.
    respond(200, '{"revision":"abc","proxy":{}}')
    await request('/api/v1/proxies/alpha', {
      method: 'PUT',
      body: {},
      ifMatch: '0123456789abcdef',
    })

    const headers = lastInit?.headers as Record<string, string>
    expect(headers['If-Match']).toBe('"0123456789abcdef"')
  })
})
