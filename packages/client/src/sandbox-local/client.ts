/**
 * Node-only local Sandbox satisfier.
 *
 * This module depends on `node:child_process` and must not be imported
 * into browser bundles. It provisions one long-lived local subprocess
 * per sandbox handle and speaks a newline-delimited JSON protocol over
 * stdin/stdout:
 *
 * request  -> `{"tool_name":"echo","arguments":{"text":"hi"}}\n`
 * response -> `{"kind":"ok","value":{"text":"hi"}}\n`
 *
 * ## Usage
 *
 * ```ts
 * import { createLocalSandbox } from '@fireline/client/sandbox-local'
 *
 * const sandbox = createLocalSandbox({
 *   workerCommand: ['node', './tool-worker.mjs'],
 *   workingDir: process.cwd(),
 * })
 *
 * const handle = await sandbox.provision({ runtime_key: 'runtime:test' })
 * const result = await sandbox.execute(handle, {
 *   tool_name: 'echo',
 *   arguments: { text: 'hello' },
 * })
 * ```
 */
import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process'

import type { JsonValue } from '../core/index.js'
import type {
  Sandbox,
  SandboxHandle,
  SandboxSpec,
  ToolResult,
} from '../sandbox/index.js'

export interface LocalSandboxOptions {
  readonly workerCommand: readonly string[]
  readonly workingDir?: string
  readonly env?: Readonly<Record<string, string>>
}

type PendingExecution = {
  readonly callId: string
  readonly resolve: (line: string) => void
  readonly reject: (error: Error) => void
}

type SandboxRecord = {
  readonly child: ChildProcessWithoutNullStreams
  readonly allowedTools?: ReadonlySet<string>
  readonly stderrLines: string[]
  readonly bufferedStdoutLines: string[]
  stdoutBuffer: string
  currentExecution?: PendingExecution
  exitCode: number | null
  exitSignal: NodeJS.Signals | null
  spawnError?: string
}

const STDERR_RING_SIZE = 20
const STOP_TIMEOUT_MS = 5_000

export function createLocalSandbox(opts: LocalSandboxOptions): Sandbox {
  if (opts.workerCommand.length === 0) {
    throw new Error('createLocalSandbox requires a non-empty workerCommand')
  }

  const sandboxes = new Map<string, SandboxRecord>()

  return {
    async provision(spec) {
      const handle: SandboxHandle = {
        id: crypto.randomUUID(),
        kind: 'local-subprocess',
      }

      const child = spawn(opts.workerCommand[0], opts.workerCommand.slice(1), {
        cwd: opts.workingDir,
        env: {
          ...process.env,
          ...opts.env,
          FIRELINE_SANDBOX_ID: handle.id,
          FIRELINE_RUNTIME_KEY: spec.runtime_key,
          ...(spec.mount_paths?.length ? { FIRELINE_MOUNT_PATHS: JSON.stringify(spec.mount_paths) } : {}),
        },
        stdio: ['pipe', 'pipe', 'pipe'],
      })

      const record: SandboxRecord = {
        child,
        allowedTools: readAllowedTools(spec),
        stderrLines: [],
        bufferedStdoutLines: [],
        stdoutBuffer: '',
        exitCode: null,
        exitSignal: null,
      }

      sandboxes.set(handle.id, record)
      wireSandboxProcess(handle.id, record)

      return handle
    },

    async execute(handle, call) {
      const record = sandboxes.get(handle.id)
      if (!record) {
        return { kind: 'error', message: `unknown sandbox handle '${handle.id}'` }
      }

      if (record.spawnError) {
        return { kind: 'error', message: record.spawnError }
      }

      if (!isChildAlive(record)) {
        return { kind: 'error', message: describeExitedSandbox(handle.id, record) }
      }

      if (record.allowedTools && !record.allowedTools.has(call.tool_name)) {
        return {
          kind: 'error',
          message: `tool '${call.tool_name}' is not registered for sandbox '${handle.id}'`,
        }
      }

      if (record.currentExecution) {
        return {
          kind: 'error',
          message: `sandbox '${handle.id}' is already executing call '${record.currentExecution.callId}'`,
        }
      }

      const unexpectedStdout = record.bufferedStdoutLines.shift()
      if (unexpectedStdout) {
        return {
          kind: 'error',
          message: `sandbox worker emitted unexpected stdout while idle: ${unexpectedStdout}`,
        }
      }

      const line = await executeViaChild(record, {
        callId: call.call_id ?? crypto.randomUUID(),
        payload: `${JSON.stringify({ tool_name: call.tool_name, arguments: call.arguments })}\n`,
      }).catch((error: unknown) => {
        const message = error instanceof Error ? error.message : String(error)
        return `__FIRELINE_SANDBOX_ERROR__:${message}`
      })

      if (line.startsWith('__FIRELINE_SANDBOX_ERROR__:')) {
        return { kind: 'error', message: line.slice('__FIRELINE_SANDBOX_ERROR__:'.length) }
      }

      return normalizeToolResult(line)
    },

    async status(handle) {
      const record = sandboxes.get(handle.id)
      if (!record) {
        return { kind: 'stopped' }
      }

      if (record.spawnError) {
        return { kind: 'error', message: record.spawnError }
      }

      if (record.currentExecution) {
        return { kind: 'executing', call_id: record.currentExecution.callId }
      }

      if (isChildAlive(record)) {
        return { kind: 'ready' }
      }

      if (record.exitCode === 0 && record.exitSignal === null) {
        return { kind: 'stopped' }
      }

      return { kind: 'error', message: describeExitedSandbox(handle.id, record) }
    },

    async stop(handle) {
      const record = sandboxes.get(handle.id)
      if (!record) {
        return
      }

      rejectPendingExecution(record, new Error(`sandbox '${handle.id}' stopped`))

      if (!isChildAlive(record)) {
        return
      }

      record.child.kill('SIGTERM')

      try {
        await waitForChildExit(record.child, STOP_TIMEOUT_MS)
      } catch {
        if (isChildAlive(record)) {
          record.child.kill('SIGKILL')
        }
        await waitForChildExit(record.child, STOP_TIMEOUT_MS).catch(() => undefined)
      }
    },
  }

  function wireSandboxProcess(handleId: string, record: SandboxRecord): void {
    record.child.stdout.setEncoding('utf8')
    record.child.stdout.on('data', (chunk: string) => {
      record.stdoutBuffer += chunk

      while (true) {
        const newlineIndex = record.stdoutBuffer.indexOf('\n')
        if (newlineIndex === -1) {
          break
        }

        const line = record.stdoutBuffer.slice(0, newlineIndex).trim()
        record.stdoutBuffer = record.stdoutBuffer.slice(newlineIndex + 1)
        if (!line) {
          continue
        }

        if (record.currentExecution) {
          const pending = record.currentExecution
          record.currentExecution = undefined
          pending.resolve(line)
          continue
        }

        record.bufferedStdoutLines.push(line)
      }
    })

    record.child.stderr.setEncoding('utf8')
    record.child.stderr.on('data', (chunk: string) => {
      pushStderr(record.stderrLines, chunk)
    })

    record.child.once('error', (error) => {
      record.spawnError = `sandbox worker failed to start: ${error.message}`
      rejectPendingExecution(record, new Error(record.spawnError))
    })

    record.child.once('exit', (code, signal) => {
      record.exitCode = code
      record.exitSignal = signal
      rejectPendingExecution(record, new Error(describeExitedSandbox(handleId, record)))
    })
  }
}

function readAllowedTools(spec: SandboxSpec): ReadonlySet<string> | undefined {
  if (!spec.capabilities || spec.capabilities.length === 0) {
    return undefined
  }

  return new Set(spec.capabilities.map((capability) => capability.descriptor.name))
}

async function executeViaChild(
  record: SandboxRecord,
  options: {
    readonly callId: string
    readonly payload: string
  },
): Promise<string> {
  const response = new Promise<string>((resolve, reject) => {
    record.currentExecution = {
      callId: options.callId,
      resolve,
      reject,
    }
  })

  const child = record.child
  if (child.stdin.destroyed || child.stdin.writableEnded) {
    rejectPendingExecution(record, new Error('sandbox worker stdin is closed'))
    return response
  }

  try {
    child.stdin.write(options.payload)
  } catch (error) {
    rejectPendingExecution(
      record,
      error instanceof Error ? error : new Error(String(error)),
    )
  }

  return response
}

function normalizeToolResult(line: string): ToolResult {
  try {
    const parsed = JSON.parse(line) as unknown
    if (isErrorResult(parsed)) {
      return { kind: 'error', message: parsed.message }
    }
    if (isOkResult(parsed)) {
      return { kind: 'ok', value: parsed.value }
    }
    return { kind: 'ok', value: parsed as JsonValue }
  } catch (error) {
    return {
      kind: 'error',
      message: error instanceof Error ? `sandbox worker emitted invalid JSON: ${error.message}` : String(error),
    }
  }
}

function isOkResult(value: unknown): value is Extract<ToolResult, { kind: 'ok' }> {
  return typeof value === 'object' && value !== null && 'kind' in value && value.kind === 'ok' && 'value' in value
}

function isErrorResult(value: unknown): value is Extract<ToolResult, { kind: 'error' }> {
  return (
    typeof value === 'object' &&
    value !== null &&
    'kind' in value &&
    value.kind === 'error' &&
    'message' in value &&
    typeof value.message === 'string'
  )
}

function pushStderr(lines: string[], chunk: string): void {
  for (const line of chunk.split(/\r?\n/)) {
    const trimmed = line.trim()
    if (!trimmed) {
      continue
    }
    lines.push(trimmed)
  }

  if (lines.length > STDERR_RING_SIZE) {
    lines.splice(0, lines.length - STDERR_RING_SIZE)
  }
}

function rejectPendingExecution(record: SandboxRecord, error: Error): void {
  if (!record.currentExecution) {
    return
  }

  const pending = record.currentExecution
  record.currentExecution = undefined
  pending.reject(error)
}

function isChildAlive(record: SandboxRecord): boolean {
  return record.exitCode === null && record.exitSignal === null && !record.spawnError
}

function describeExitedSandbox(handleId: string, record: SandboxRecord): string {
  const stderrSuffix =
    record.stderrLines.length > 0 ? ` stderr: ${record.stderrLines.join(' | ')}` : ''

  if (record.spawnError) {
    return `${record.spawnError}${stderrSuffix}`
  }

  if (record.exitSignal) {
    return `sandbox '${handleId}' exited via signal ${record.exitSignal}.${stderrSuffix}`.trim()
  }

  if (record.exitCode === 0) {
    return `sandbox '${handleId}' has stopped.${stderrSuffix}`.trim()
  }

  return `sandbox '${handleId}' exited with code ${record.exitCode ?? 'unknown'}.${stderrSuffix}`.trim()
}

async function waitForChildExit(
  child: ChildProcessWithoutNullStreams,
  timeoutMs: number,
): Promise<{ code: number | null; signal: NodeJS.Signals | null }> {
  if (child.exitCode !== null || child.signalCode !== null) {
    return {
      code: child.exitCode,
      signal: child.signalCode,
    }
  }

  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      reject(new Error(`timed out waiting for sandbox worker ${child.pid ?? 'unknown'} to exit`))
    }, timeoutMs)

    child.once('exit', (code, signal) => {
      clearTimeout(timeout)
      resolve({ code, signal })
    })
  })
}
