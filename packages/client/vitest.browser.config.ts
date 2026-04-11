/// <reference types="@vitest/browser/providers/playwright" />

import { existsSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { defineConfig } from 'vitest/config'

const chromeExecutablePath = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'
const mockBrowserHarness = process.env.MOCK_BROWSER_HARNESS === 'true'
const mockFirelineState = fileURLToPath(
  new URL('./test/browser-harness.mock-fireline-state.ts', import.meta.url),
)
const mockReactDb = fileURLToPath(
  new URL('./test/browser-harness.mock-react-db.ts', import.meta.url),
)

export default defineConfig({
  resolve: {
    alias: mockBrowserHarness
      ? {
          '@fireline/state': mockFirelineState,
          '@tanstack/react-db': mockReactDb,
        }
      : undefined,
  },
  server: {
    fs: {
      allow: ['..'],
    },
    proxy: {
      '/api': {
        target: 'http://127.0.0.1:4436',
      },
      '/acp': {
        target: 'http://127.0.0.1:4437',
        ws: true,
      },
      '/v1': {
        target: 'http://127.0.0.1:4437',
      },
      '/healthz': {
        target: 'http://127.0.0.1:4437',
      },
    },
  },
  test: {
    include: [
      'test/browser-acp.browser.test.ts',
      '../browser-harness/test/**/*.browser.test.ts',
    ],
    exclude: ['**/node_modules/**', '**/dist/**', '**/__screenshots__/**'],
    globalSetup: ['./test/browser.global-setup.ts'],
    browser: {
      enabled: true,
      name: 'chromium',
      provider: 'playwright',
      providerOptions: existsSync(chromeExecutablePath)
        ? {
            launch: {
              executablePath: chromeExecutablePath,
            },
          }
        : {},
      headless: true,
    },
  },
})
