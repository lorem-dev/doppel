import type { Config } from 'jest'

// `@swc/jest` rather than `ts-jest`: ts-jest pins `typescript <7` and needs
// `@babel/core` as a peer, and this transform needs neither. Types are checked
// by `npm run typecheck`, not by the test run -- one job each.
const config: Config = {
  testEnvironment: 'jsdom',
  setupFilesAfterEnv: ['<rootDir>/src/test-setup.ts'],
  transform: {
    '^.+\\.(t|j)sx?$': [
      '@swc/jest',
      {
        jsc: {
          parser: { syntax: 'typescript', tsx: true },
          transform: { react: { runtime: 'automatic' } },
        },
      },
    ],
  },
  // react-router is deliberately absent from these suites. It ships ESM only and
  // uses `import.meta`, which cannot survive a transform to CommonJS, and running
  // jest in ESM mode would change how every mock in this project is written. So
  // anything that renders a link or navigates is covered by the Playwright suite,
  // against a real browser and the real router -- which is the stronger test of
  // navigation regardless. What stays here is what is genuinely a unit: services,
  // stores, and the components that do not route.
  moduleNameMapper: {
    // Styles are a vite concern; under jest they resolve to nothing rather than
    // failing to parse.
    '\\.css$': '<rootDir>/src/__mocks__/style.ts',
  },
  testMatch: ['<rootDir>/src/**/*.test.ts', '<rootDir>/src/**/*.test.tsx'],
}

export default config
