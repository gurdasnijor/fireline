import { execFile, spawn, type ChildProcessWithoutNullStreams } from 'node:child_process'
import { access } from 'node:fs/promises'
import { constants } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

import { afterAll, beforeAll, describe, expect, it } from 'vitest'

const packageDir = dirname(fileURLToPath(new URL('../package.json', import.meta.url)))
const harnessUrl = 'http://127.0.0.1:5173'
const harnessDriverUrl = `${harnessUrl}/e2e.html`
const harnessApiUrl = 'http://127.0.0.1:4436/api/agents'
const defaultChromePath = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'
const browserSession = `fireline-browser-harness-vitest-${Date.now()}`
const viteBin = fileURLToPath(new URL('../node_modules/vite/bin/vite.js', import.meta.url))
const devServerScript = fileURLToPath(new URL('../dev-server.mjs', import.meta.url))

let controlServer: ChildProcessWithoutNullStreams | undefined
let viteServer: ChildProcessWithoutNullStreams | undefined

describe('browser harness e2e', () => {
  beforeAll(async () => {
    controlServer = startProcess('control', process.execPath, [devServerScript])
    await waitForHttpOk(harnessApiUrl)

    viteServer = startProcess('vite', process.execPath, [viteBin, '--host', '--strictPort'])
    await waitForHttpOk(harnessUrl)
  }, 30_000)

  afterAll(async () => {
    await closeBrowser()
    await stopProcess(viteServer)
    await stopProcess(controlServer)
  })

  it(
    'prompts over ACP from a real browser context and observes durable state over StreamDB',
    async () => {
      await agentBrowser(['open', harnessDriverUrl])
      await waitForEval('Boolean(window.firelineE2E?.run)')

      const result = await agentBrowserJson<{
        runtimeId: string
        runtimeStatus: string
        sessionId: string
        stopReason: string
        promptText: string
        promptTurnText: string | null
        chunkContent: string | null
        supportsLoadSession: boolean | null
      }>(['eval', '(async () => await window.firelineE2E.run())()'])

      expect(result.runtimeStatus).toBe('ready')
      expect(result.runtimeId).toMatch(/^fireline:browser-harness:/)
      expect(result.sessionId).toMatch(
        /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i,
      )
      expect(result.stopReason).toBe('end_turn')
      expect(result.supportsLoadSession).toBe(true)
      expect(result.promptTurnText).toBe(result.promptText)
      expect(result.chunkContent).toContain('Hello')
    },
    60_000,
  )
})

function startProcess(
  name: string,
  command: string,
  args: string[],
): ChildProcessWithoutNullStreams {
  const child = spawn(command, args, {
    cwd: packageDir,
    env: process.env,
    stdio: ['ignore', 'pipe', 'pipe'],
  })

  for (const stream of [child.stdout, child.stderr]) {
    stream.on('data', (chunk) => {
      process.stdout.write(`[${name}] ${chunk.toString('utf8')}`)
    })
  }

  return child
}

async function waitForHttpOk(url: string, timeoutMs = 20_000): Promise<void> {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    try {
      const response = await fetch(url)
      if (response.ok) {
        return
      }
    } catch {
      // Keep polling until the server is listening.
    }

    await sleep(100)
  }

  throw new Error(`timed out waiting for ${url}`)
}

async function waitForEval(expression: string, timeoutMs = 10_000): Promise<void> {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    try {
      const result = await agentBrowserJson<boolean>(['eval', expression])
      if (result) {
        return
      }
    } catch {
      // Keep polling until the page finishes booting.
    }

    await sleep(100)
  }

  throw new Error(`timed out waiting for browser expression: ${expression}`)
}

async function agentBrowser(args: string[]): Promise<string> {
  const commandArgs = ['--session', browserSession]

  if (await isExecutable(defaultChromePath)) {
    commandArgs.push('--executable-path', defaultChromePath)
  }

  return await execFileText('agent-browser', [...commandArgs, ...args])
}

async function agentBrowserJson<T>(args: string[]): Promise<T> {
  const output = await agentBrowser(['--json', ...args])
  const parsed = JSON.parse(output) as {
    success: boolean
    data?: { result?: T }
    error?: string | null
  }

  if (!parsed.success) {
    throw new Error(parsed.error ?? 'agent-browser command failed')
  }

  return parsed.data?.result as T
}

async function closeBrowser(): Promise<void> {
  try {
    await agentBrowser(['close'])
  } catch {
    // The browser session may not exist if setup failed.
  }
}

async function stopProcess(child: ChildProcessWithoutNullStreams | undefined): Promise<void> {
  if (!child || child.exitCode !== null || child.signalCode !== null) {
    return
  }

  child.kill('SIGTERM')
  await Promise.race([
    new Promise<void>((resolve) => {
      child.once('exit', () => resolve())
    }),
    sleep(5_000),
  ])
}

async function isExecutable(path: string): Promise<boolean> {
  try {
    await access(path, constants.X_OK)
    return true
  } catch {
    return false
  }
}

async function execFileText(command: string, args: string[]): Promise<string> {
  return await new Promise((resolve, reject) => {
    execFile(command, args, { cwd: packageDir }, (error, stdout, stderr) => {
      if (error) {
        reject(new Error(stderr.trim() || stdout.trim() || error.message))
        return
      }

      resolve(stdout)
    })
  })
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, ms)
  })
}
