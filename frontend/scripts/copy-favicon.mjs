// The favicon comes from `assets/` at the repository root, the same directory the
// documentation build stages into `docs/assets/`. Copied rather than committed
// under `public/` so there is one source of truth for the icon; the copy is
// git-ignored.
//
// Node's own `fs` rather than a vite plugin: a plugin would be a dependency for
// two lines, and `CONTRIBUTING.md` asks what a new one does that the standard
// library cannot.
import { copyFileSync, mkdirSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const source = resolve(here, '../../assets/favicon.ico')
const target = resolve(here, '../public/favicon.ico')

mkdirSync(dirname(target), { recursive: true })
copyFileSync(source, target)
console.log(`favicon: ${source} -> ${target}`)
