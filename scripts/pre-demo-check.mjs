#!/usr/bin/env node

import { spawn } from 'node:child_process'
import net from 'node:net'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url))
const REPO_ROOT = path.resolve(SCRIPT_DIR, '..')
const DIVIDER = '='.repeat(64)
const PORTS = [4436, 4437, 4440, 5173]
const SUBPATH_EXPORTS = [
  '@fireline/client',
  '@fireline/client/middleware',
  '@fireline/client/admin',
  '@fireline/client/events',
  '@fireline/client/resources',
]

class CheckFailure extends Error {
  constructor(message, options = {}) {
    super(message)
    this.name = 'CheckFailure'
    this.details = options.details ?? []
    this.diagnosticLabel = options.diagnosticLabel ?? 'Details'
    this.diagnosticText = options.diagnosticText ?? message
  }
}

const CHECKS = [
  { id: 1, label: 'Git state (clean, in sync with origin)', run: checkGitState },
  { id: 2, label: 'cargo check --workspace', run: checkCargoWorkspace },
  { id: 3, label: '@fireline/client build', run: checkClientBuild },
  { id: 4, label: '@fireline/client subpath exports', run: checkClientSubpathExports },
  { id: 5, label: 'Port availability (4436, 4437, 4440, 5173)', run: checkPortAvailability },
]

async function main() {
  const results = []
  let failure = null

  for (const check of CHECKS) {
    const startedAt = Date.now()

    try {
      const outcome = await check.run()
      results.push({
        id: check.id,
        label: check.label,
        status: 'PASS',
        durationMs: Date.now() - startedAt,
        details: outcome.details ?? [],
      })
    } catch (error) {
      const normalized = normalizeFailure(error)
      const failedResult = {
        id: check.id,
        label: check.label,
        status: 'FAIL',
        durationMs: Date.now() - startedAt,
        details: normalized.details,
        diagnosticLabel: normalized.diagnosticLabel,
        diagnosticText: normalized.diagnosticText,
      }

      results.push(failedResult)
      failure = failedResult
      break
    }
  }

  for (const check of CHECKS.slice(results.length)) {
    results.push({
      id: check.id,
      label: check.label,
      status: 'SKIP',
      details: ['Not run because an earlier check failed.'],
    })
  }

  printSummary(results)

  if (failure) {
    printFailureBlock(failure)
    process.exitCode = 1
    return
  }

  process.exitCode = 0
}

async function checkGitState() {
  const statusResult = await runCommand('git', ['status', '--short'])
  ensureSuccess('git status --short', statusResult)

  const workingTree = statusResult.stdout.trim()
  if (workingTree) {
    throw new CheckFailure('working tree is not clean', {
      details: ['`git status --short` returned local changes.'],
      diagnosticLabel: 'git status --short',
      diagnosticText: statusResult.stdout.trim(),
    })
  }

  const headResult = await runCommand('git', ['rev-parse', 'HEAD'])
  ensureSuccess('git rev-parse HEAD', headResult)

  const upstreamResult = await runCommand('git', ['rev-parse', '@{u}'])
  ensureSuccess('git rev-parse @{u}', upstreamResult)

  const headSha = headResult.stdout.trim()
  const upstreamSha = upstreamResult.stdout.trim()
  if (headSha !== upstreamSha) {
    throw new CheckFailure('HEAD is not in sync with upstream', {
      details: [`HEAD ${headSha}`, `Upstream ${upstreamSha}`],
      diagnosticLabel: 'Git SHA mismatch',
      diagnosticText: `HEAD ${headSha}\nUPSTREAM ${upstreamSha}`,
    })
  }

  return {
    details: [`HEAD ${headSha}`],
  }
}

async function checkCargoWorkspace() {
  await runCheckedCommand('cargo check --workspace', ['cargo', 'check', '--workspace'])
  return { details: [] }
}

async function checkClientBuild() {
  await runCheckedCommand('@fireline/client build', ['pnpm', '--filter', '@fireline/client', 'build'])
  return { details: [] }
}

async function checkClientSubpathExports() {
  const probeScript = `
const specifiers = ${JSON.stringify(SUBPATH_EXPORTS)};
const summary = [];
for (const specifier of specifiers) {
  const namespace = await import(specifier);
  const exportsSummary = Object.keys(namespace)
    .sort()
    .map((name) => ({ name, type: typeof namespace[name] }));
  summary.push({ specifier, exports: exportsSummary });
}
process.stdout.write(JSON.stringify(summary));
`

  const result = await runCommand(process.execPath, [
    '--input-type=module',
    '--eval',
    probeScript,
  ])
  ensureSuccess('@fireline/client subpath export probe', result)

  let summary
  try {
    summary = JSON.parse(result.stdout)
  } catch (error) {
    throw new CheckFailure('subpath export probe emitted invalid JSON', {
      diagnosticLabel: 'Subpath probe stdout',
      diagnosticText:
        result.stdout.trim() ||
        (error instanceof Error ? error.message : String(error)),
    })
  }

  const details = summary.map((entry) => `${entry.specifier} (${entry.exports.length} exports)`)
  return { details }
}

async function checkPortAvailability() {
  const occupied = []

  for (const port of PORTS) {
    const result = await probePort(port)
    if (!result.available) {
      occupied.push(result)
    }
  }

  if (occupied.length > 0) {
    const details = occupied.map((entry) =>
      entry.reason ? `Port ${entry.port}: ${entry.reason}` : `Port ${entry.port}: in use`,
    )
    throw new CheckFailure('one or more demo ports are occupied', {
      details,
      diagnosticLabel: 'Port probe',
      diagnosticText: details.join('\n'),
    })
  }

  return {
    details: [`All ports free: ${PORTS.join(', ')}`],
  }
}

async function runCheckedCommand(label, [command, ...args]) {
  const result = await runCommand(command, args)
  ensureSuccess(label, result)
  return result
}

async function runCommand(command, args) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: REPO_ROOT,
      env: process.env,
      stdio: ['ignore', 'pipe', 'pipe'],
    })

    let stdout = ''
    let stderr = ''
    let settled = false

    child.stdout.setEncoding('utf8')
    child.stdout.on('data', (chunk) => {
      stdout += chunk
    })

    child.stderr.setEncoding('utf8')
    child.stderr.on('data', (chunk) => {
      stderr += chunk
    })

    child.once('error', (error) => {
      if (settled) {
        return
      }
      settled = true
      reject(
        new CheckFailure(`failed to spawn ${command}`, {
          diagnosticLabel: `${command} spawn error`,
          diagnosticText: error instanceof Error ? error.message : String(error),
        }),
      )
    })

    child.once('close', (code, signal) => {
      if (settled) {
        return
      }
      settled = true
      resolve({ code, signal, stdout, stderr })
    })
  })
}

function ensureSuccess(label, result) {
  if (result.code === 0 && result.signal === null) {
    return
  }

  const diagnosticText =
    result.stderr.trim() ||
    result.stdout.trim() ||
    `${label} exited with code ${result.code ?? 'unknown'}${result.signal ? ` (signal ${result.signal})` : ''}`

  throw new CheckFailure(`${label} failed`, {
    diagnosticLabel: `${label} stderr`,
    diagnosticText,
  })
}

async function probePort(port) {
  return new Promise((resolve) => {
    const server = net.createServer()
    let settled = false

    const finish = (value) => {
      if (settled) {
        return
      }
      settled = true
      resolve(value)
    }

    server.once('error', (error) => {
      const reason =
        error && typeof error === 'object' && 'code' in error
          ? `${error.code}: ${error.message}`
          : String(error)
      finish({ port, available: false, reason })
    })

    server.listen(port, () => {
      server.close((closeError) => {
        if (closeError) {
          finish({ port, available: false, reason: closeError.message })
          return
        }
        finish({ port, available: true })
      })
    })
  })
}

function normalizeFailure(error) {
  if (error instanceof CheckFailure) {
    return error
  }

  return new CheckFailure(error instanceof Error ? error.message : String(error))
}

function printSummary(results) {
  console.log(DIVIDER)
  console.log(' Pre-Demo Check Summary')
  console.log(DIVIDER)

  for (const result of results) {
    console.log(formatSummaryLine(result))
    for (const detail of result.details ?? []) {
      console.log(`     - ${detail}`)
    }
  }

  console.log(DIVIDER)

  const passed = results.filter((result) => result.status === 'PASS').length
  const failed = results.find((result) => result.status === 'FAIL')

  if (failed) {
    console.log(`${passed}/${results.length} checks passed - stopped after failure at [${failed.id}]`)
  } else {
    console.log(`${passed}/${results.length} checks PASSED - demo environment is green`)
  }

  console.log(DIVIDER)
}

function printFailureBlock(failure) {
  console.error('')
  console.error(`Failure diagnostic for [${failure.id}] ${failure.label}`)
  console.error(DIVIDER)
  console.error(failure.diagnosticLabel)
  console.error(DIVIDER)
  console.error(failure.diagnosticText)
  console.error(DIVIDER)
}

function formatSummaryLine(result) {
  const prefix = `[${result.id}] ${result.label}`.padEnd(52)
  const status = result.status.padEnd(4)
  const duration = result.durationMs != null ? ` (${formatDuration(result.durationMs)})` : ''
  return `${prefix} ${status}${duration}`
}

function formatDuration(durationMs) {
  if (durationMs < 1000) {
    return `${durationMs}ms`
  }

  if (durationMs < 10_000) {
    return `${(durationMs / 1000).toFixed(1)}s`
  }

  return `${Math.round(durationMs / 1000)}s`
}

await main()
