import type { FirelineAgent } from './agent.js'
import type { Harness, StartOptions } from './types.js'

export interface NamedTopology<Names extends string> {
  start(options: StartOptions): Promise<Record<Names, FirelineAgent<Names>>>
}

export interface FanoutTopology<Name extends string> {
  start(options: StartOptions): Promise<Array<FirelineAgent<Name>>>
}

export function peer<const T extends readonly Harness<string>[]>(
  ...harnesses: T
): NamedTopology<T[number]['name']> {
  return {
    async start(options) {
      const stateStream = options.stateStream ?? `fireline-peer-${crypto.randomUUID()}`
      const handles = await Promise.all(
        harnesses.map((harness) =>
          harness.start({ ...options, name: harness.name, stateStream }),
        ),
      )
      return Object.fromEntries(
        handles.map((handle) => [handle.name, handle]),
      ) as Record<T[number]['name'], FirelineAgent<T[number]['name']>>
    },
  }
}

export function fanout<const H extends Harness<string>>(
  harness: H,
  options: { readonly count: number },
): FanoutTopology<H['name']> {
  return {
    async start(startOptions) {
      return Promise.all(
        Array.from({ length: options.count }, (_, index) =>
          harness.start({
            ...startOptions,
            name: `${startOptions.name ?? harness.name}-${index + 1}`,
          }),
        ),
      )
    },
  }
}

export function pipe<const T extends readonly Harness<string>[]>(
  ...harnesses: T
): NamedTopology<T[number]['name']> {
  return {
    async start(options) {
      const stateStream = options.stateStream ?? `fireline-pipe-${crypto.randomUUID()}`
      const handles: FirelineAgent<string>[] = []
      for (const harness of harnesses) {
        handles.push(await harness.start({ ...options, name: harness.name, stateStream }))
      }
      return Object.fromEntries(
        handles.map((handle) => [handle.name, handle]),
      ) as Record<T[number]['name'], FirelineAgent<T[number]['name']>>
    },
  }
}
