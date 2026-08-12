// The configurations the specs run against.
//
// Ports and the template directory are placeholders the harness fills in, so two
// spec files running at once cannot collide.

export const ROOT_TOKEN = 'root-token-0000000000000000000000000'
export const READER_TOKEN = 'reader-token-00000000000000000000000'

/** Tokens for both a full-rights caller and a read-only one. */
const TOKENS = `
  tokens:
    - name: root
      group: admin
      token: ${ROOT_TOKEN}
    - name: reader
      group: user
      token: ${READER_TOKEN}
`

const PROXIES = `
proxies:
  - name: alpha
    type: http
    url: "https://alpha.example.com/api/"
  - name: beta
    type: http
    url: "https://beta.example.com/api/"
    # Resolved by header, because only one proxy may be the default one -- rule
    # V12, and a fixture that broke it would fail every spec at startup.
    resolve:
      type: header
      header: X-Proxy-Name
`

/** Reads are public, writes need `admin`: the shape most deployments have. */
export const PRIVATE_CONFIG = `
server:
  host: "127.0.0.1"
  port: {proxyPort}
admin:
  host: "127.0.0.1"
  port: {adminPort}
  title: "Doppel (e2e)"
${TOKENS}
  access:
    list: public
    read: public
    create: ["admin"]
    update: ["admin"]
    delete: ["admin"]
    upload: ["admin"]
  upload:
    limit: 1Mi
control:
  socket: {controlSocket}
templates:
  dir: {templatesDir}
${PROXIES}
`

/** Everything unauthenticated: the page must not ask for a token at all. */
export const PUBLIC_CONFIG = `
server:
  host: "127.0.0.1"
  port: {proxyPort}
admin:
  host: "127.0.0.1"
  port: {adminPort}
  public: true
  tokens: []
  access: {}
  upload:
    limit: 1Mi
control:
  socket: {controlSocket}
templates:
  dir: {templatesDir}
${PROXIES}
`

/**
 * Public, with a proxy whose `access` block the public API makes inert.
 *
 * The configuration an operator reaches by turning `public: true` on over proxies
 * they had already restricted: the overrides stay in the document and stop
 * deciding anything, which the form has to say rather than offer fields for.
 * `alpha` holds one and `beta` holds none, so one instance covers both answers.
 */
export const PUBLIC_OVERRIDES_CONFIG = `
server:
  host: "127.0.0.1"
  port: {proxyPort}
admin:
  host: "127.0.0.1"
  port: {adminPort}
  public: true
  tokens: []
  access: {}
  upload:
    limit: 1Mi
control:
  socket: {controlSocket}
templates:
  dir: {templatesDir}
proxies:
  - name: alpha
    type: http
    url: "https://alpha.example.com/api/"
    access:
      delete: public
  - name: beta
    type: http
    url: "https://beta.example.com/api/"
    resolve:
      type: header
      header: X-Proxy-Name
`

/** The dashboard turned off: the API stays, the three routes go. */
export const DASHBOARD_OFF_CONFIG = `
server:
  host: "127.0.0.1"
  port: {proxyPort}
admin:
  host: "127.0.0.1"
  port: {adminPort}
  dashboard: false
${TOKENS}
  access:
    list: public
    read: public
    create: ["admin"]
    update: ["admin"]
    delete: ["admin"]
    upload: ["admin"]
  upload:
    limit: 1Mi
control:
  socket: {controlSocket}
templates:
  dir: {templatesDir}
${PROXIES}
`
