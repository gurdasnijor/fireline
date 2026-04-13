const TRUE_VALUES = new Set(['1', 'true', 'yes', 'on', 'repl'])

export function logReplDebug(event: string, details?: Record<string, unknown>): void {
  if (!isReplDebugEnabled()) {
    return
  }

  const payload =
    details && Object.keys(details).length > 0
      ? ` ${JSON.stringify(details, debugReplacer)}`
      : ''
  process.stderr.write(`FL-DEBUG ${event}${payload}\n`)
}

function isReplDebugEnabled(): boolean {
  const raw =
    process.env.FIRELINE_REPL_DEBUG ??
    process.env.FL_DEBUG ??
    process.env.FL_REPL_DEBUG
  if (!raw) {
    return false
  }

  return raw
    .split(',')
    .map((value) => value.trim().toLowerCase())
    .some((value) => TRUE_VALUES.has(value))
}

function debugReplacer(_key: string, value: unknown): unknown {
  if (value instanceof Error) {
    return { message: value.message, name: value.name }
  }
  return value
}
