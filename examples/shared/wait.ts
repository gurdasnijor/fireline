export async function waitForRows<T>(
  collection: {
    readonly toArray: readonly T[]
    subscribeChanges(callback: () => void): { unsubscribe(): void }
  },
  predicate: (rows: readonly T[]) => boolean,
  timeoutMs: number,
): Promise<readonly T[]> {
  const snapshot = () => [...collection.toArray]
  if (predicate(snapshot())) return snapshot()
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => { sub.unsubscribe(); reject(new Error(`timed out after ${timeoutMs}ms`)) }, timeoutMs)
    const sub = collection.subscribeChanges(() => {
      const rows = snapshot()
      if (!predicate(rows)) return
      clearTimeout(timeout)
      sub.unsubscribe()
      resolve(rows)
    })
  })
}
