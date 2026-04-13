import type { Awakeable, AwakeableResolution } from './awakeable.js'
import { awakeableResolutionSymbol } from './awakeable.js'
import type { CompletionKey, WorkflowTraceContext } from './keys.js'

export interface AwakeableRaceWinner<T> {
  readonly winnerIndex: number
  readonly winnerKey: CompletionKey
  readonly value: T
  readonly traceContext?: WorkflowTraceContext
}

type InternalAwakeable<T> = Awakeable<T> & {
  readonly [awakeableResolutionSymbol]: Promise<AwakeableResolution<T>>
}

/**
 * Promise.race-style sugar over multiple awakeables.
 *
 * This is strictly additive: it composes over the existing awakeable promises
 * and carries forward the trace context from the winning completion envelope.
 */
export async function raceAwakeables<T>(
  awakeables: Iterable<Awakeable<T>>,
): Promise<AwakeableRaceWinner<T>> {
  const branches = Array.from(awakeables).map((awakeable, winnerIndex) => {
    const resolution = (awakeable as InternalAwakeable<T>)[awakeableResolutionSymbol]
    if (!resolution) {
      throw new Error('raceAwakeables requires awakeables created by workflowContext()')
    }

    return resolution.then((winner) => ({
      winnerIndex,
      winnerKey: winner.key,
      value: winner.value,
      traceContext: winner.traceContext,
    }))
  })

  if (branches.length === 0) {
    throw new Error('awakeable race requires at least one branch')
  }

  return Promise.race(branches)
}
