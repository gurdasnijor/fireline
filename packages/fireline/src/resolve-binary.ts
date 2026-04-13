import { existsSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

export type BinaryName = 'fireline' | 'fireline-streams' | 'fireline-agents'
export type BinarySource = 'env' | 'release' | 'debug'

export interface BinaryLookup {
  /** Name of the executable. */
  readonly name: BinaryName
  /** Env var that overrides the lookup. */
  readonly envVar: string
  /** Optional start directory for walking up to `target/{release,debug}`. */
  readonly searchFrom?: string
}

export interface ResolvedBinary {
  readonly name: BinaryName
  readonly path: string
  readonly source: BinarySource
}

export class BinaryResolutionError extends Error {
  readonly kind: 'env-missing' | 'not-found'
  readonly lookup: BinaryLookup

  constructor(
    kind: 'env-missing' | 'not-found',
    lookup: BinaryLookup,
    message: string,
  ) {
    super(message)
    this.name = 'BinaryResolutionError'
    this.kind = kind
    this.lookup = lookup
  }
}

const DEFAULT_SEARCH_FROM = dirname(fileURLToPath(import.meta.url))

export function findBinary(lookup: BinaryLookup): ResolvedBinary | null {
  const envOverride = process.env[lookup.envVar]
  if (envOverride && existsSync(envOverride)) {
    return {
      name: lookup.name,
      path: envOverride,
      source: 'env',
    }
  }
  if (envOverride) {
    throw new BinaryResolutionError(
      'env-missing',
      lookup,
      `${lookup.envVar}=${envOverride} but no binary exists at that path`,
    )
  }

  const searchFrom = lookup.searchFrom ?? DEFAULT_SEARCH_FROM
  for (const source of ['release', 'debug'] as const) {
    const candidate = findTargetBinary(lookup.name, source, searchFrom)
    if (candidate) {
      return {
        name: lookup.name,
        path: candidate,
        source,
      }
    }
  }

  return null
}

export function resolveBinary(lookup: BinaryLookup): ResolvedBinary {
  const resolved = findBinary(lookup)
  if (resolved) {
    return resolved
  }
  throw new BinaryResolutionError(
    'not-found',
    lookup,
    [
      `Could not find '${lookup.name}'.`,
      `Tried ${lookup.envVar}, target/release/${lookup.name}, and target/debug/${lookup.name}.`,
      `Fix: run 'cargo build --release --bin ${lookup.name}' from the fireline workspace root,`,
      `or set ${lookup.envVar} to an absolute path.`,
    ].join('\n'),
  )
}

function findTargetBinary(
  name: BinaryName,
  source: Exclude<BinarySource, 'env'>,
  searchFrom: string,
): string | null {
  let dir = searchFrom
  for (let i = 0; i < 10; i++) {
    const candidate = resolve(dir, 'target', source, name)
    if (existsSync(candidate)) {
      return candidate
    }
    const parent = dirname(dir)
    if (parent === dir) {
      break
    }
    dir = parent
  }
  return null
}
