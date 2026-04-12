export async function waitFor<T>(
  getValue: () => T | undefined,
  timeoutMs: number,
  intervalMs = 50,
): Promise<T> {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const value = getValue()
    if (value !== undefined) {
      return value
    }
    await sleep(intervalMs)
  }
  throw new Error(`timed out after ${timeoutMs}ms`)
}

export function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}
