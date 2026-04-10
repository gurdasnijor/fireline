/// <reference types="@vitest/browser/providers/playwright" />

import { existsSync } from 'node:fs'
import { defineConfig } from 'vitest/config'

const chromeExecutablePath = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'

export default defineConfig({
  server: {
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
    include: ['test/browser-acp.browser.test.ts'],
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
