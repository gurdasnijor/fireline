import { setTimeout as delay } from 'node:timers/promises'
import { fileURLToPath } from 'node:url'

import { describe, expect, it } from 'vitest'

import { createLocalSandbox } from '../src/sandbox-local/index.js'
import type { SandboxHandle } from '../src/sandbox/index.js'

const stubWorkerPath = fileURLToPath(new URL('./fixtures/stub-sandbox-worker.mjs', import.meta.url))

describe('sandbox-local', () => {
  it('provisions, executes, reports ready, and stops a local sandbox worker', async () => {
    const sandbox = createLocalSandbox({
      workerCommand: ['node', stubWorkerPath],
    })

    let handle: SandboxHandle | undefined

    try {
      handle = await sandbox.provision({ runtime_key: 'test-rk' })

      expect(handle.kind).toBe('local-subprocess')
      expect(handle.id).toMatch(/\S+/)

      expect(await sandbox.status(handle)).toEqual({ kind: 'ready' })

      const first = await sandbox.execute(handle, {
        tool_name: 'echo',
        arguments: { message: 'first', count: 1 },
      })
      expect(first).toEqual({
        kind: 'ok',
        value: { echoed: { message: 'first', count: 1 } },
      })

      expect(await sandbox.status(handle)).toEqual({ kind: 'ready' })

      const second = await sandbox.execute(handle, {
        tool_name: 'echo',
        arguments: { message: 'second', count: 2 },
      })
      expect(second).toEqual({
        kind: 'ok',
        value: { echoed: { message: 'second', count: 2 } },
      })
      expect(second).not.toEqual(first)

      await Promise.race([
        sandbox.stop(handle),
        delay(4_500).then(() => {
          throw new Error('sandbox.stop did not complete before SIGTERM timeout')
        }),
      ])

      expect(await sandbox.status(handle)).toEqual({ kind: 'stopped' })
    } finally {
      if (handle) {
        await sandbox.stop(handle)
      }
    }
  })
})
