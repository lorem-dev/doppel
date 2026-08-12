import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import { defineConfig } from 'vite'

export default defineConfig({
  // Every emitted URL is `/static/...`, which is where the admin listener serves
  // the embedded assets from. `index.html` itself is served from `/` with the
  // runtime configuration substituted into it, so it is not under this prefix.
  base: '/static/',
  plugins: [react(), tailwindcss()],
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
