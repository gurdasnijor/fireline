const TOP_LEVEL_COMMANDS = new Set(['run', 'build', 'deploy', 'agents', 'repl'])

export function normalizePastelArgv(argv: readonly string[]): readonly string[] {
  const normalized = argv.length >= 2 ? [...argv] : ['node', 'fireline', ...argv]
  const firstArg = normalized[2]

  if (
    !firstArg ||
    firstArg.startsWith('-') ||
    TOP_LEVEL_COMMANDS.has(firstArg)
  ) {
    return normalized
  }

  return [normalized[0]!, normalized[1]!, 'run', ...normalized.slice(2)]
}

export function pushBooleanFlag(argv: string[], flag: string, enabled: boolean): void {
  if (enabled) {
    argv.push(flag)
  }
}

export function pushNumberFlag(
  argv: string[],
  flag: string,
  value: number | undefined,
): void {
  if (typeof value === 'number') {
    argv.push(flag, String(value))
  }
}

export function pushStringFlag(
  argv: string[],
  flag: string,
  value: string | undefined,
): void {
  if (value) {
    argv.push(flag, value)
  }
}
