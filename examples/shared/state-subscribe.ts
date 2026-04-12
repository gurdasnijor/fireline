export function waitForRows<T>(
  collection: {
    readonly toArray: readonly T[]
    subscribeChanges(listener: () => void): { unsubscribe(): void } | (() => void) | void
  },
  predicate: (rows: readonly T[]) => boolean,
  timeoutMs: number,
): Promise<readonly T[]> {
  return new Promise((resolve, reject) => {
    const check = () => {
      if (predicate(collection.toArray)) {
        cleanup()
        resolve(collection.toArray)
      }
    }
    const timeout = setTimeout(() => {
      cleanup()
      reject(new Error(`timed out after ${timeoutMs}ms`))
    }, timeoutMs)
    const unsubscribe = collection.subscribeChanges(check)
    const cleanup = () => {
      clearTimeout(timeout)
      if (typeof unsubscribe === 'function') {
        unsubscribe()
      } else if (unsubscribe && typeof unsubscribe === 'object' && 'unsubscribe' in unsubscribe) {
        unsubscribe.unsubscribe()
      }
    }
    check()
  })
}
