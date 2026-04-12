import {
  Sandbox,
  agent,
  compose,
  middleware,
  sandbox,
} from '../../packages/client/src/sandbox.ts'
import type { Harness, HarnessHandle, StartOptions } from '../../packages/client/src/types.ts'

export { Sandbox, agent, compose, middleware, sandbox }

type NamedHandles<T extends readonly Harness[]> = {
  [K in T[number] as K['name']]: HarnessHandle<K['name']>
}

export function peer<const T extends readonly Harness[]>(...harnesses: T) {
  return createTopology('peer', harnesses)
}

export function pipe<const T extends readonly Harness[]>(...harnesses: T) {
  return createTopology('pipe', harnesses)
}

export function fanout<const H extends Harness>(harness: H, opts: { count: number }) {
  return {
    async start(options: StartOptions) {
      return Promise.all(
        Array.from({ length: opts.count }, (_, index) =>
          harness.as(`${harness.name}-${index}`).start({
            ...options,
            name: `${options.name ?? harness.name}-${index}`,
          }),
        ),
      )
    },
  }
}

function createTopology<const T extends readonly Harness[]>(kind: 'peer' | 'pipe', harnesses: T) {
  return {
    async start(options: StartOptions): Promise<NamedHandles<T>> {
      const stateStream = options.stateStream ?? `${kind}-${Date.now()}`
      const entries = await Promise.all(
        harnesses.map(async (harness) => [
          harness.name,
          await harness.start({
            ...options,
            name: options.name ? `${options.name}-${harness.name}` : harness.name,
            stateStream,
          }),
        ]),
      )
      return Object.fromEntries(entries) as NamedHandles<T>
    },
  }
}
