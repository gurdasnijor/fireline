import type { Awakeable } from './awakeable.js'

export class AwakeableTimeoutError extends Error {
  constructor() {
    super(
      'awakeable timeout requires DS Phase 6 WakeTimerSubscriber; Phase 5 only publishes the API signature',
    )
    this.name = 'AwakeableTimeoutError'
  }
}

/**
 * Signature-only timeout helper. Real wake-timer append/consume behavior lands
 * with DS Phase 6; until then this remains an explicit blocker instead of
 * emitting write-only timer events.
 */
export async function withAwakeableTimeout<T>(
  _awakeable: Awakeable<T>,
  _durationMs: number,
): Promise<T> {
  throw new AwakeableTimeoutError()
}
