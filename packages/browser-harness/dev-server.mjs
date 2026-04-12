import { createServer } from 'node:http'
import { spawn } from 'node:child_process'
import { mkdir } from 'node:fs/promises'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const packageDir = dirname(fileURLToPath(import.meta.url))
const repoRoot = dirname(dirname(packageDir))
const tmpDir = join(packageDir, '.tmp')
const firelineBin = join(repoRoot, 'target', 'debug', 'fireline')
const firelineStreamsBin = join(repoRoot, 'target', 'debug', 'fireline-streams')
const firelineTestyLoadBin = join(repoRoot, 'target', 'debug', 'fireline-testy-load')
const controlPlaneUrl = 'http://127.0.0.1:4440'
const preferPush = process.env.PREFER_PUSH === 'true'
const durableStreamsUrl = process.env.DURABLE_STREAMS_URL ?? 'http://127.0.0.1:7474/v1/stream'

const agents = [
  {
    source: 'local',
    id: 'fireline-testy-load',
    name: 'Fireline Testy Load',
    version: 'local',
    description: 'Local Fireline proof agent with loadSession support',
    launchable: true,
    distributionKind: 'command',
  },
]

const resolvedCommands = new Map([
  ['fireline-testy-load', [firelineTestyLoadBin]],
])

let controlPlaneProcess = null
let streamsProcess = null

await mkdir(tmpDir, { recursive: true })

if (!process.env.DURABLE_STREAMS_URL) {
  await startEmbeddedStreams()
}

await startControlPlane()

const server = createServer(async (req, res) => {
  try {
    if (!req.url) {
      sendJson(res, 400, { error: 'missing_url' })
      return
    }

    const url = new URL(req.url, 'http://127.0.0.1:4436')

    if (req.method === 'GET' && url.pathname === '/api/agents') {
      sendJson(res, 200, { agents })
      return
    }

    if (req.method === 'GET' && url.pathname === '/api/resolve') {
      const agentId = url.searchParams.get('agentId')
      if (!agentId) {
        sendJson(res, 400, { error: 'missing_agent_id' })
        return
      }
      const resolved = resolvedCommands.get(agentId)
      if (!resolved) {
        sendJson(res, 404, { error: `unknown_agent_id: ${agentId}` })
        return
      }
      sendJson(res, 200, { agentCommand: resolved })
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

async function shutdown() {
  server.close()
  await stopControlPlane()
  await stopEmbeddedStreams()
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

async function startEmbeddedStreams() {
  console.log('starting embedded durable-streams server on port 7474')

  streamsProcess = spawn(
    firelineStreamsBin,
    [],
    {
      stdio: ['ignore', 'pipe', 'pipe'],
      env: { ...process.env, PORT: '7474' },
    },
  )

  for (const stream of [streamsProcess.stdout, streamsProcess.stderr]) {
    stream?.on('data', (chunk) => {
      process.stdout.write(`[streams] ${chunk.toString('utf8')}`)
    })
  }

  await waitForHttpReady('http://127.0.0.1:7474/healthz', streamsProcess)
  console.log('durable-streams ready at http://127.0.0.1:7474/v1/stream')
}

async function stopEmbeddedStreams() {
  if (!streamsProcess) {
    return
  }
  const child = streamsProcess
  streamsProcess = null
  if (child.exitCode !== null || child.signalCode !== null) {
    return
  }
  child.kill('SIGTERM')
  await new Promise((resolve) => child.on('close', resolve))
}

async function startControlPlane() {
  console.log(
    `starting fireline --control-plane with prefer_push=${preferPush ? 'true' : 'false'}`,
  )

  controlPlaneProcess = spawn(
    firelineBin,
    [
      '--control-plane',
      '--host',
      '127.0.0.1',
      '--port',
      '4440',
      '--durable-streams-url',
      durableStreamsUrl,
      '--fireline-bin',
      firelineBin,
      '--startup-timeout-ms',
      '20000',
      '--stop-timeout-ms',
      '10000',
    ],
    {
      stdio: ['ignore', 'pipe', 'pipe'],
      env: process.env,
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
