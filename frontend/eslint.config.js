// Flat config, eslint 10. Three layers and no more: the language defaults,
// typescript-eslint's recommended set, and the React hook rules -- which are the
// ones that catch real bugs rather than style.
import js from '@eslint/js'
import reactHooks from 'eslint-plugin-react-hooks'
import tseslint from 'typescript-eslint'

export default tseslint.config(
  { ignores: ['dist', 'coverage', 'playwright-report', 'test-results', 'node_modules'] },
  {
    // The build helpers run under node, not in a browser, so `console` and
    // `process` are globals there and nowhere else.
    files: ['scripts/**/*.mjs', '*.config.{js,ts}'],
    languageOptions: { globals: { console: 'readonly', process: 'readonly' } },
  },
  js.configs.recommended,
  tseslint.configs.recommended,
  {
    files: ['**/*.{ts,tsx}'],
    plugins: { 'react-hooks': reactHooks },
    rules: { ...reactHooks.configs.recommended.rules },
  },
  {
    files: ['src/components/**/*.tsx'],
    rules: {
      // The layer rule, stated where a machine can check it: a presentational
      // component knows nothing about the API. Pages do -- a page is the layer
      // that fetches, holds the screen's state and hands data down -- and shared
      // state that outlives one screen lives in a store instead. Without this
      // rule the first component to grow a `fetch` would be the one nothing can
      // be tested without.
      'no-restricted-imports': [
        'error',
        {
          patterns: [
            {
              group: ['**/services/*', '!**/services/runtimeConfig'],
              message:
                'a presentational component must not call the API; a page or a store does that',
            },
          ],
        },
      ],
    },
  },
)
