#!/usr/bin/env node
import fs from 'node:fs'
import fsp from 'node:fs/promises'
import path from 'node:path'
import { spawn } from 'node:child_process'
import process from 'node:process'

import fireline, {
  appendApprovalResolved,
  connectAcp,
} from '../../../packages/client/dist/index.js'

for (const method of ['log', 'warn', 'error']) {
  const original = console[method].bind(console)
  console[method] = (...args) => {
    if (typeof args[0] === 'string' && args[0].startsWith('[StreamDB]')) {
      return
    }
    original(...args)
  }
}

const __dirname = path.dirname(new URL(import.meta.url).pathname)
const repoRoot = path.resolve(__dirname, '../../..')
const artifactRoot = process.env.ARTIFACT_ROOT ?? path.join(repoRoot, '.tmp/fqa-approval-demo')
const runDir = process.env.RUN_DIR ?? path.join(artifactRoot, 'latest')
const logDir = path.join(runDir, 'logs')
const cliPath = process.env.CLI_ENTRY ?? path.join(repoRoot, 'packages/fireline/dist/cli.js')
const harnessSpec = process.env.HARNESS_SPEC ?? path.join(repoRoot, 'docs/demos/scripts/fqa-approval-harness.ts')

const port = process.env.CONTROL_PORT ?? '4540'
const streamsPort = process.env.STREAMS_PORT ?? '7574'
const stateStream = process.env.STATE_STREAM ?? 'fqa-approval-public'

const modes = new Set(['full', 'driver-only'])
const argv = process.argv.slice(2)
const mode = modes.has(argv[0] ?? '') ? argv.shift() : 'full'
const args = parseArgs(argv)

await fsp.mkdir(logDir, { recursive: true })

function log(message) {
  process.stdout.write(`[fqa-approval] ${message}\n`)
}

function parseArgs(input) {
  const out = {
    acpUrl: null,
    stateUrl: null,
    timeoutMs: 15000,
  }

  for (let i = 0; i < input.length; i++) {
    const arg = input[i]
    switch (arg) {
      case '--acp-url':
        out.acpUrl = input[++i]
        break
      case '--state-url':
        out.stateUrl = input[++i]
        break
      case '--timeout-ms':
        out.timeoutMs = Number(input[++i])
        break
      default:
        throw new Error(`unknown arg: ${arg}`)
    }
  }

  return out
}

function stripAnsi(text) {
  return text.replace(/\u001b\[[0-9;]*m/g, '')
}

function parseReadyFields(text) {
  const clean = stripAnsi(text)
  const acpMatch = clean.match(/ACP:\s+(ws:\/\/\S+)/)
  const stateMatch = clean.match(/state:\s+(http:\/\/\S+)/)
  return {
    acpUrl: acpMatch?.[1] ?? null,
    stateUrl: stateMatch?.[1] ?? null,
  }
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

async function waitFor(getValue, timeoutMs) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const value = await getValue()
    if (value !== undefined && value !== null) return value
    await sleep(100)
  }
  return null
}

function chunkText(row) {
  const update = row?.update
  if (!update || typeof update !== 'object') return ''
  const content = update.content
  return typeof content?.text === 'string' ? content.text : ''
}

async function waitForPermission(db, sessionId, timeoutMs) {
  return await waitFor(
    async () =>
      db.permissions.toArray.find(
        (row) => row.sessionId === sessionId && row.state === 'pending',
      ) ?? null,
    timeoutMs,
  )
}

async function waitForChunk(db, sessionId, needle, timeoutMs) {
  return await waitFor(
    async () =>
      db.chunks.toArray.find(
        (row) => row.sessionId === sessionId && chunkText(row).includes(needle),
      ) ?? null,
    timeoutMs,
  )
}

function serialize(value) {
  return JSON.parse(JSON.stringify(value))
}

async function runScenario({ acpUrl, stateUrl, timeoutMs }) {
  const db = await fireline.db({ stateStreamUrl: stateUrl })
  const acp = await connectAcp(acpUrl, 'fqa-approval-demo')

  async function runPrompt({ name, promptText, allow, expectChunk }) {
    const { sessionId } = await acp.newSession({ cwd: '/workspace', mcpServers: [] })
    const promptPromise = acp.prompt({
      sessionId,
      prompt: [{ type: 'text', text: promptText }],
    })
    const permission = await waitForPermission(db, sessionId, timeoutMs)
    let promptResponse = null
    let promptError = null
    let chunk = null

    if (permission) {
      await appendApprovalResolved({
        streamUrl: stateUrl,
        sessionId,
        requestId: permission.requestId,
        allow,
        resolvedBy: 'fqa-approval-demo',
      })
    }

    try {
      promptResponse = serialize(await promptPromise)
    } catch (error) {
      promptError = error instanceof Error ? error.message : String(error)
    }

    if (expectChunk) {
      chunk = serialize(await waitForChunk(db, sessionId, expectChunk, timeoutMs))
    } else {
      chunk = serialize(await waitForChunk(db, sessionId, '/workspace/fqa-denied.txt', 2000))
    }

    const verdict = allow
      ? permission && promptResponse && chunk ? 'pass' : 'fail'
      : permission && promptError && !chunk ? 'pass' : 'fail'

    return {
      name,
      verdict,
      sessionId,
      requestId: permission?.requestId ?? null,
      permissionTitle: permission?.title ?? null,
      promptResponse,
      promptError,
      chunk,
    }
  }

  try {
    const allowResult = await runPrompt({
      name: 'approval-allow',
      promptText: JSON.stringify({
        command: 'write_file',
        path: '/workspace/fqa-approved.txt',
        content: 'approved write from public replay',
      }),
      allow: true,
      expectChunk: 'ok:/workspace/fqa-approved.txt',
    })

    const denyResult = await runPrompt({
      name: 'approval-deny',
      promptText: JSON.stringify({
        command: 'write_file',
        path: '/workspace/fqa-denied.txt',
        content: 'denied write from public replay',
      }),
      allow: false,
      expectChunk: null,
    })

    const summary = {
      sourceReview: 'docs/reviews/fqa-approval-session-2026-04-12.md',
      summaryPath: path.join(runDir, 'summary.json'),
      executedAt: new Date().toISOString(),
      publicSurface: {
        cliBootstrap: `node packages/fireline/dist/cli.js run docs/demos/scripts/fqa-approval-harness.ts --port ${port} --streams-port ${streamsPort} --state-stream ${stateStream}`,
        clientDriver: 'node docs/demos/scripts/replay-fqa-approval.mjs driver-only --acp-url <ws-url> --state-url <http-url>',
      },
      endpoints: {
        acpUrl,
        stateUrl,
      },
      scenarioResults: [
        allowResult,
        denyResult,
      ],
      limitations: {
        promptLevelFallback: [allowResult, denyResult].some((row) =>
          String(row.permissionTitle ?? '').includes('fallback'),
        ),
        deniedPathReturnedGenericInternalError: String(denyResult.promptError ?? '').includes('Internal error'),
        publicSurfaceCoversCrashRestartSessionLoad: false,
      },
      findings: {
        sourceReviewCrashRestartSessionLoad: {
          reproducibleFromAdvertisedSurfaceToday: false,
          note: 'The advertised CLI/public-client path covers approval allow/deny, but not the original FQA crash/restart/session/load leg. That still requires an internal host/process-control path and remains a truthful gap.',
        },
      },
    }

    await fsp.writeFile(summary.summaryPath, `${JSON.stringify(summary, null, 2)}\n`)
    process.stdout.write(`${JSON.stringify({
      summaryPath: summary.summaryPath,
      acpUrl,
      stateUrl,
      allowVerdict: allowResult.verdict,
      denyVerdict: denyResult.verdict,
      promptLevelFallback: summary.limitations.promptLevelFallback,
      publicSurfaceCoversCrashRestartSessionLoad: summary.limitations.publicSurfaceCoversCrashRestartSessionLoad,
    }, null, 2)}\n`)
  } finally {
    await acp.close().catch(() => {})
    db.close()
  }
}

async function runFull() {
  const logPath = path.join(logDir, 'fireline-cli.log')
  const logStream = fs.createWriteStream(logPath, { flags: 'w' })
  const child = spawn(
    process.execPath,
    [cliPath, 'run', harnessSpec, '--port', port, '--streams-port', streamsPort, '--state-stream', stateStream],
    {
      cwd: repoRoot,
      env: {
        ...process.env,
        FIRELINE_BIN: process.env.FIRELINE_BIN ?? path.join(repoRoot, 'target/debug/fireline'),
        FIRELINE_STREAMS_BIN: process.env.FIRELINE_STREAMS_BIN ?? path.join(repoRoot, 'target/debug/fireline-streams'),
      },
      stdio: ['ignore', 'pipe', 'pipe'],
    },
  )

  let transcript = ''

  const onData = (chunk, stream) => {
    const text = chunk.toString('utf8')
    transcript += text
    logStream.write(text)
    stream.write(text)
  }

  child.stdout.on('data', (chunk) => onData(chunk, process.stdout))
  child.stderr.on('data', (chunk) => onData(chunk, process.stderr))

  const ready = await waitFor(async () => {
    const parsed = parseReadyFields(transcript)
    return parsed.acpUrl && parsed.stateUrl ? parsed : null
  }, args.timeoutMs)

  if (!ready) {
    child.kill('SIGINT')
    throw new Error('timed out waiting for CLI ready output')
  }

  try {
    await runScenario({
      acpUrl: ready.acpUrl,
      stateUrl: ready.stateUrl,
      timeoutMs: args.timeoutMs,
    })
  } finally {
    if (!child.killed && child.exitCode === null) {
      child.kill('SIGINT')
    }
    await new Promise((resolve) => child.once('exit', resolve))
    logStream.end()
  }
}

if (mode === 'driver-only') {
  if (!args.acpUrl || !args.stateUrl) {
    throw new Error('driver-only mode requires --acp-url and --state-url')
  }
  await runScenario({
    acpUrl: args.acpUrl,
    stateUrl: args.stateUrl,
    timeoutMs: args.timeoutMs,
  })
} else {
  log('starting surfaced CLI + public-client approval replay')
  await runFull()
}
