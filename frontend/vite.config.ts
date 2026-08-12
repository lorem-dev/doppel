import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import { minify } from 'html-minifier-terser'
import { defineConfig, type Plugin } from 'vite'

/**
 * Minify `index.html`, which vite emits as it was written.
 *
 * It is the one file the listener serves on every page load and cannot cache
 * immutably -- the assets are content-hashed and cached for a year, this is
 * substituted per request -- so the comments and indentation in it are paid for
 * by every visit.
 *
 * Deliberately conservative. Two things in this file are load-bearing and would
 * fail quietly if minification touched them:
 *
 * - `id="doppel-config"`, with double quotes. `dashboard.rs` finds the element
 *   by that exact string and splices the runtime configuration into it, and
 *   `build.rs` asserts the string is present. `removeAttributeQuotes` is off
 *   (it is off by default, and named here so nobody turns it on).
 * - The `</script>` that closes it, which is the other half of the splice.
 *
 * `minifyJS` stays off as well: the element is `application/json`, not a script,
 * and handing JSON to a JS minifier is a way to find out what it does with it.
 */
function minifyHtml(): Plugin {
  return {
    name: 'doppel:minify-html',
    apply: 'build',
    transformIndexHtml: {
      // After vite has rewritten the asset URLs, so what is minified is what
      // ships.
      order: 'post',
      handler: (html) =>
        minify(html, {
          collapseWhitespace: true,
          removeComments: true,
          removeAttributeQuotes: false,
          minifyJS: false,
          minifyCSS: false,
        }),
    },
  }
}

export default defineConfig({
  // Every emitted URL is `/static/...`, which is where the admin listener serves
  // the embedded assets from. `index.html` itself is served from `/` with the
  // runtime configuration substituted into it, so it is not under this prefix.
  base: '/static/',
  plugins: [react(), tailwindcss(), minifyHtml()],
  build: {
    outDir: 'dist',
    // The requirement is the smallest static payload that will do the job, and a
    // sourcemap roughly doubles it. A stack trace from a minified admin page is
    // a worse trade than a megabyte on the wire.
    sourcemap: false,
    // Filenames are content-hashed, which is what makes the immutable
    // `Cache-Control` the listener sends safe.
    assetsDir: 'assets',
  },
})
