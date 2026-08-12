// A real Doppel, per spec file.
//
// Not Playwright's `webServer`: that starts one process for the whole run, and
// these specs need different configurations -- public, private, dashboard off --
// which is a property of the server rather than of the page. Each file starts its
// own on ports nobody else is using.

import { spawn, type ChildProcess } from 'node:child_process'
import { createServer } from 'node:net'
import { mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'

export interface Doppel {
  /** Where the admin listener, and therefore the dashboard, answers. */
  baseURL: string
  /**
   * Where proxied traffic goes.
   *
   * Only the metrics spec needs it, and it needs it for a real reason: a process
   * that has served no request has no series to expose, so an exposition test
   * with no traffic behind it asserts nothing.
   */
  proxyURL: string
  stop: () => void
}

/** A port the operating system has just confirmed is free. */
async function freePort(): Promise<number> {
  return new Promise((accept, reject) => {
    const probe = createServer()
    probe.once('error', reject)
    probe.listen(0, '127.0.0.1', () => {
      const address = probe.address()
      if (typeof address === 'string' || address === null) {
        reject(new Error('no port'))
        return
      }
      const { port } = address
      probe.close(() => accept(port))
    })
  })
}

/**
 * Start Doppel with `yaml`, with `{proxyPort}`, `{adminPort}` and
 * `{templatesDir}` substituted.
 *
 * The binary comes from `DOPPEL_BIN` or `target/debug/doppel`, and it has to have
 * been built with `frontend/dist` in place -- otherwise `/` answers 503 and every
 * spec here fails for the same uninformative reason, which is why the wait below
 * checks the dashboard rather than only the API.
 */
export async function startDoppel(yaml: string): Promise<Doppel> {
  const dir = mkdtempSync(join(tmpdir(), 'doppel-e2e-'))
  const proxyPort = await freePort()
  const adminPort = await freePort()
  const templatesDir = join(dir, 'templates')

  const configPath = join(dir, 'main.yaml')
  writeFileSync(
    configPath,
    yaml
      .replaceAll('{proxyPort}', String(proxyPort))
      .replaceAll('{adminPort}', String(adminPort))
      .replaceAll('{templatesDir}', templatesDir)
      // Short, and outside the temp directory: a unix socket path is capped near
      // 104 bytes on macOS and the temp directory alone can spend most of that.
      // Per-instance because the default is one path for every Doppel on the
      // machine, and spec files run in parallel -- the second server would exit
      // 1 with the first still holding the socket.
      .replaceAll('{controlSocket}', `/tmp/doppel-e2e-${adminPort}.sock`),
  )

  const binary = process.env.DOPPEL_BIN ?? resolve(import.meta.dirname, '../../../target/debug/doppel')
  const child: ChildProcess = spawn(binary, ['serve', '--config', configPath], {
    stdio: ['ignore', 'pipe', 'pipe'],
  })

  // Kept, and printed only when the wait fails: a server that refused its own
  // configuration says why on stderr, and without this the failure is a timeout
  // with no cause.
  let log = ''
  child.stdout?.on('data', (chunk: Buffer) => (log += chunk.toString()))
  child.stderr?.on('data', (chunk: Buffer) => (log += chunk.toString()))

  const baseURL = `http://127.0.0.1:${adminPort}`
  const stop = () => child.kill('SIGTERM')

  const deadline = Date.now() + 20_000
  for (;;) {
    if (child.exitCode !== null) {
      throw new Error(`doppel exited with ${child.exitCode}:\n${log}`)
    }
    try {
      const response = await fetch(`${baseURL}/api/v1/status`)
      if (response.ok) {
        return { baseURL, proxyURL: `http://127.0.0.1:${proxyPort}`, stop }
      }
    } catch {
      // Not up yet.
    }
    if (Date.now() > deadline) {
      stop()
      throw new Error(`doppel did not start within 20s:\n${log}`)
    }
    await new Promise((accept) => setTimeout(accept, 100))
  }
}
