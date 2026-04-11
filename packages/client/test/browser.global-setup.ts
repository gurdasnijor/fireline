import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process'
import { fileURLToPath } from 'node:url'

import type { GlobalSetupContext } from 'vitest/node'

const browserHarnessDir = fileURLToPath(new URL('../../browser-harness/', import.meta.url))
const devServerScript = fileURLToPath(new URL('../../browser-harness/dev-server.mjs', import.meta.url))

export default async function browserGlobalSetup(_context: GlobalSetupContext) {
  if (process.env.MOCK_BROWSER_HARNESS === 'true') {
    return async () => {}
  }

  const server = spawn(process.execPath, [devServerScript], {
    cwd: browserHarnessDir,
    env: process.env,
    stdio: ['ignore', 'pipe', 'pipe'],
  })

  for (const stream of [server.stdout, server.stderr]) {
    stream.on('data', (chunk) => {
      process.stdout.write(`[browser-global-setup] ${chunk.toString('utf8')}`)
    })
  }

  await waitForHttpOk('http://127.0.0.1:4436/api/agents')

  return async () => {
    await stopProcess(server)
  }
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
      // Keep polling until the control server is listening.
    }

    await sleep(100)
  }

  throw new Error(`timed out waiting for ${url}`)
}

async function stopProcess(child: ChildProcessWithoutNullStreams): Promise<void> {
  if (child.exitCode !== null || child.signalCode !== null) {
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

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, ms)
  })
}
