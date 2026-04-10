import { createServer } from 'node:http'
import { spawn } from 'node:child_process'
import { mkdir } from 'node:fs/promises'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

import { createFirelineClient } from '@fireline/client'

const packageDir = dirname(fileURLToPath(import.meta.url))
const repoRoot = dirname(dirname(packageDir))
const tmpDir = join(packageDir, '.tmp')
const runtimeRegistryPath = join(tmpDir, 'runtimes.toml')
const peerDirectoryPath = join(tmpDir, 'peers.toml')
const firelineBin = join(repoRoot, 'target', 'debug', 'fireline')
const firelineControlPlaneBin = join(repoRoot, 'target', 'debug', 'fireline-control-plane')
const firelineTestyLoadBin = join(repoRoot, 'target', 'debug', 'fireline-testy-load')
const controlPlaneUrl = 'http://127.0.0.1:4440'
const preferPush = process.env.PREFER_PUSH === 'true'

const client = createFirelineClient({
  host: {
    controlPlaneUrl,
  },
  catalog: {
    localEntries: [
      {
        source: 'local',
        id: 'fireline-testy-load',
        name: 'Fireline Testy Load',
        version: 'local',
        description: 'Local Fireline proof agent with loadSession support',
        distributions: [
          {
            kind: 'command',
            command: [firelineTestyLoadBin],
          },
        ],
      },
    ],
  },
})

let currentRuntime = null
let controlPlaneProcess = null

await mkdir(tmpDir, { recursive: true })
await startControlPlane()

const server = createServer(async (req, res) => {
  try {
    if (!req.url) {
      sendJson(res, 400, { error: 'missing_url' })
      return
    }

    const url = new URL(req.url, 'http://127.0.0.1:4436')

    if (req.method === 'GET' && url.pathname === '/api/agents') {
      const agents = await client.catalog.listAgents()
      const items = await Promise.all(
        agents.map(async (agent) => {
          try {
            const launch = await client.catalog.resolveAgent(agent.id)
            return {
              ...agent,
              launchable: true,
              distributionKind: launch.distributionKind,
            }
          } catch (error) {
            return {
              ...agent,
              launchable: false,
              unavailableReason: toErrorMessage(error),
            }
          }
        }),
      )
      sendJson(res, 200, { agents: items })
      return
    }

    if (req.method === 'GET' && url.pathname === '/api/runtime') {
      if (!currentRuntime) {
        sendJson(res, 200, { runtime: null })
        return
      }
      const runtime = await client.host.get(currentRuntime.runtimeKey)
      currentRuntime = runtime
      sendJson(res, 200, { runtime })
      return
    }

    if (req.method === 'POST' && url.pathname === '/api/runtime') {
      const body = await readJson(req)
      const agentId = typeof body?.agentId === 'string' ? body.agentId : null
      if (!agentId) {
        sendJson(res, 400, { error: 'missing_agent_id' })
        return
      }

      await stopCurrentRuntime()

      currentRuntime = await client.host.create({
        provider: 'local',
        host: '127.0.0.1',
        port: 4437,
        name: 'browser-harness',
        stateStream: 'fireline-harness-state',
        peerDirectoryPath,
        agent: {
          source: 'catalog',
          agentId,
        },
      })

      sendJson(res, 200, { runtime: currentRuntime })
      return
    }

    if (req.method === 'DELETE' && url.pathname === '/api/runtime') {
      await stopCurrentRuntime()
      sendJson(res, 200, { runtime: null })
      return
    }

    sendJson(res, 404, { error: 'not_found' })
  } catch (error) {
    sendJson(res, 500, { error: toErrorMessage(error) })
  }
})

server.listen(4436, '127.0.0.1', () => {
  console.log('browser harness control server ready on http://127.0.0.1:4436')
})

for (const signal of ['SIGINT', 'SIGTERM']) {
  process.on(signal, async () => {
    await shutdown()
    process.exit(0)
  })
}

async function stopCurrentRuntime() {
  if (!currentRuntime) {
    return
  }

  try {
    await client.host.delete(currentRuntime.runtimeKey)
  } finally {
    currentRuntime = null
  }
}

async function shutdown() {
  server.close()
  await stopCurrentRuntime()
  await client.close()
  await stopControlPlane()
}

function sendJson(res, statusCode, payload) {
  const body = JSON.stringify(payload)
  res.writeHead(statusCode, {
    'content-type': 'application/json; charset=utf-8',
    'content-length': Buffer.byteLength(body),
    'cache-control': 'no-store',
  })
  res.end(body)
}

async function readJson(req) {
  const chunks = []
  for await (const chunk of req) {
    chunks.push(chunk)
  }
  if (chunks.length === 0) {
    return null
  }
  return JSON.parse(Buffer.concat(chunks).toString('utf8'))
}

function toErrorMessage(error) {
  if (error instanceof Error) {
    return error.message
  }
  return String(error)
}

async function startControlPlane() {
  console.log(
    `starting fireline-control-plane with prefer_push=${preferPush ? 'true' : 'false'}`,
  )

  controlPlaneProcess = spawn(
    firelineControlPlaneBin,
    [
      '--host',
      '127.0.0.1',
      '--port',
      '4440',
      '--fireline-bin',
      firelineBin,
      '--runtime-registry-path',
      runtimeRegistryPath,
      '--peer-directory-path',
      peerDirectoryPath,
      '--startup-timeout-ms',
      '20000',
      '--stop-timeout-ms',
      '10000',
    ],
    {
      stdio: ['ignore', 'pipe', 'pipe'],
      env: {
        ...process.env,
        FIRELINE_CONTROL_PLANE_PREFER_PUSH: preferPush ? 'true' : 'false',
      },
    },
  )

  for (const stream of [controlPlaneProcess.stdout, controlPlaneProcess.stderr]) {
    stream?.on('data', (chunk) => {
      process.stdout.write(`[control-plane] ${chunk.toString('utf8')}`)
    })
  }

  await waitForHttpReady(`${controlPlaneUrl}/healthz`, controlPlaneProcess)
}

async function stopControlPlane() {
  if (!controlPlaneProcess) {
    return
  }

  const child = controlPlaneProcess
  controlPlaneProcess = null
  if (child.exitCode !== null || child.signalCode !== null) {
    return
  }

  child.kill('SIGTERM')
  await waitForExit(child, 5_000).catch(() => undefined)
}

async function waitForHttpReady(url, child) {
  const deadline = Date.now() + 10_000
  while (Date.now() < deadline) {
    if (child.exitCode !== null || child.signalCode !== null) {
      throw new Error(`control plane exited before becoming ready (${child.exitCode ?? child.signalCode})`)
    }

    try {
      const response = await fetch(url)
      if (response.ok) {
        return
      }
    } catch {
      // Keep polling until the server is listening.
    }

    await new Promise((resolve) => setTimeout(resolve, 100))
  }

  throw new Error('timed out waiting for control plane to become ready')
}

async function waitForExit(child, timeoutMs) {
  if (child.exitCode !== null || child.signalCode !== null) {
    return
  }

  await new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error('timed out waiting for process exit')), timeoutMs)
    child.once('exit', () => {
      clearTimeout(timeout)
      resolve()
    })
  })
}
