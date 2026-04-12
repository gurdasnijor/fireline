/**
 * Public Orchestrator primitive interfaces and the default while-loop
 * satisfier from `docs/proposals/client-primitives.md`, mapped onto the
 * substrate plan in `docs/explorations/managed-agents-mapping.md`.
 */
import type { Unsubscribe } from '../core/index.js'

export type { Unsubscribe } from '../core/index.js'

export type WakeHandler = (session_id: string) => Promise<void>

export interface Orchestrator {
  wakeOne(session_id: string): Promise<void>
  start(): Promise<void>
  stop(): Promise<void>
}

export interface SessionRegistry {
  listPending(): AsyncIterable<string>
  onPendingChange(fn: () => void): Unsubscribe
}

type Deferred = {
  readonly promise: Promise<void>
  readonly resolve: () => void
  readonly reject: (error: unknown) => void
}

function createDeferred(): Deferred {
  let resolve!: () => void
  let reject!: (error: unknown) => void
  const promise = new Promise<void>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise
    reject = rejectPromise
  })
  return { promise, resolve, reject }
}

export function whileLoopOrchestrator(opts: {
  readonly handler: WakeHandler
  readonly registry: SessionRegistry
  readonly pollIntervalMs?: number
  readonly maxConcurrent?: number
  readonly onError?: (err: unknown, session_id: string) => Promise<'retry' | 'drop'>
}): Orchestrator {
  const pollIntervalMs = opts.pollIntervalMs ?? 1_000
  const maxConcurrent = Math.max(1, opts.maxConcurrent ?? 1)

  let started = false
  let stopped = false
  let activeCount = 0
  let pollRequested = false
  let waitForPollSignal: (() => void) | null = null
  let pollLoop: Promise<void> | null = null
  let unsubscribe: Unsubscribe | null = null
  let pollLoopError: unknown = null

  const queue: string[] = []
  const requests = new Map<string, Deferred>()
  const activeTasks = new Set<Promise<void>>()

  const signalPoll = () => {
    pollRequested = true
    const signal = waitForPollSignal
    waitForPollSignal = null
    signal?.()
  }

  const waitForNextPoll = () =>
    new Promise<void>((resolve) => {
      const timer = setTimeout(() => {
        if (waitForPollSignal === wake) {
          waitForPollSignal = null
        }
        resolve()
      }, pollIntervalMs)

      const wake = () => {
        clearTimeout(timer)
        if (waitForPollSignal === wake) {
          waitForPollSignal = null
        }
        resolve()
      }

      waitForPollSignal = wake
    })

  const settleRequest = (session_id: string, action: (deferred: Deferred) => void) => {
    const deferred = requests.get(session_id)
    if (!deferred) {
      return
    }
    requests.delete(session_id)
    action(deferred)
  }

  const dispatch = () => {
    while (!stopped && activeCount < maxConcurrent && queue.length > 0) {
      const session_id = queue.shift()
      if (!session_id || !requests.has(session_id)) {
        continue
      }

      activeCount += 1

      let task!: Promise<void>
      task = (async () => {
        try {
          await opts.handler(session_id)
          settleRequest(session_id, (deferred) => deferred.resolve())
        } catch (error) {
          const action = (await opts.onError?.(error, session_id)) ?? 'drop'
          if (action === 'retry' && !stopped) {
            queue.push(session_id)
            return
          }
          settleRequest(session_id, (deferred) => deferred.reject(error))
        } finally {
          activeCount -= 1
          activeTasks.delete(task)
          dispatch()
        }
      })()

      activeTasks.add(task)
    }
  }

  const enqueue = (session_id: string): Promise<void> => {
    if (stopped) {
      return Promise.reject(new Error('Orchestrator has been stopped'))
    }

    const existing = requests.get(session_id)
    if (existing) {
      return existing.promise
    }

    const deferred = createDeferred()
    requests.set(session_id, deferred)
    queue.push(session_id)
    dispatch()
    return deferred.promise
  }

  const pollOnce = async () => {
    for await (const session_id of opts.registry.listPending()) {
      void enqueue(session_id).catch(() => {})
    }
  }

  const runLoop = async () => {
    unsubscribe = opts.registry.onPendingChange(() => {
      signalPoll()
    })

    try {
      while (started && !stopped) {
        await pollOnce()
        if (!started || stopped) {
          break
        }
        if (pollRequested) {
          pollRequested = false
          continue
        }
        await waitForNextPoll()
        pollRequested = false
      }
    } finally {
      unsubscribe?.()
      unsubscribe = null
      signalPoll()
      started = false
    }
  }

  return {
    async wakeOne(session_id) {
      await enqueue(session_id)
    },

    async start() {
      if (stopped || started) {
        return
      }

      started = true
      pollLoopError = null
      pollLoop = runLoop().catch((error) => {
        pollLoopError = error
      })
    },

    async stop() {
      if (stopped) {
        return
      }

      stopped = true
      started = false
      unsubscribe?.()
      unsubscribe = null
      signalPoll()

      if (pollLoop) {
        await pollLoop
        pollLoop = null
      }

      await Promise.allSettled(activeTasks)

      for (const [session_id, deferred] of requests.entries()) {
        requests.delete(session_id)
        deferred.reject(new Error('Orchestrator has been stopped'))
      }

      if (pollLoopError) {
        throw pollLoopError
      }
    },
  }
}

export function cronOrchestrator(_opts: {
  readonly schedule: string
  readonly handler: WakeHandler
  readonly enumerate: () => Promise<readonly string[]>
}): Orchestrator {
  throw new Error('cronOrchestrator is not implemented')
}

export function httpOrchestrator(_opts: {
  readonly handler: WakeHandler
  readonly listen: { readonly port: number; readonly path?: string }
}): Orchestrator {
  throw new Error('httpOrchestrator is not implemented')
}
