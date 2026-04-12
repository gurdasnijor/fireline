export async function waitForRows<T>(
  collection: {
    readonly toArray: readonly T[]
    subscribeChanges(callback: () => void): { unsubscribe(): void }
  },
  predicate: (rows: readonly T[]) => boolean,
  timeoutMs: number,
): Promise<readonly T[]> {
  const snapshot = () => [...collection.toArray]
  const initial = snapshot()
  if (predicate(initial)) {
    return initial
  }

  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      subscription.unsubscribe()
      reject(new Error(`timed out waiting for state rows after ${timeoutMs}ms`))
    }, timeoutMs)

    const subscription = collection.subscribeChanges(() => {
      const rows = snapshot()
      if (!predicate(rows)) {
        return
      }
      clearTimeout(timeout)
      subscription.unsubscribe()
      resolve(rows)
    })
  })
}
