import { existsSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

export type BinaryName = 'fireline' | 'fireline-streams' | 'fireline-agents'
export type BinarySource = 'env' | 'package' | 'release' | 'debug'

const PLATFORM_PACKAGES: Record<string, string> = {
  'darwin-arm64': '@fireline/cli-darwin-arm64',
  'darwin-x64': '@fireline/cli-darwin-x64',
  'linux-arm64': '@fireline/cli-linux-arm64',
  'linux-x64': '@fireline/cli-linux-x64',
  'win32-x64': '@fireline/cli-win32-x64',
}

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
const PACKAGE_REQUIRE = createRequire(import.meta.url)

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

  const packagedBinary = findPackagedBinary(lookup.name)
  if (packagedBinary) {
    return {
      name: lookup.name,
      path: packagedBinary,
      source: 'package',
    }
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
  const platformKey = `${process.platform}-${process.arch}`
  const platformPackage = PLATFORM_PACKAGES[platformKey]
  throw new BinaryResolutionError(
    'not-found',
    lookup,
    [
      `Could not find '${lookup.name}'.`,
      `Tried ${lookup.envVar}, ${platformPackage ?? `no platform package for ${platformKey}`}, target/release/${lookup.name}, and target/debug/${lookup.name}.`,
      `Fix: install @fireline/cli for your platform, run 'cargo build --release --bin ${lookup.name}' from the fireline workspace root,`,
      `or set ${lookup.envVar} to an absolute path.`,
    ].join('\n'),
  )
}

function findPackagedBinary(name: BinaryName): string | null {
  const platformPackage = PLATFORM_PACKAGES[`${process.platform}-${process.arch}`]
  if (!platformPackage) {
    return null
  }

  try {
    const pkgJsonPath = PACKAGE_REQUIRE.resolve(`${platformPackage}/package.json`)
    const candidate = join(dirname(pkgJsonPath), 'bin', name)
    if (existsSync(candidate)) {
      return candidate
    }
  } catch {
    return null
  }

  return null
}

function findTargetBinary(
  name: BinaryName,
  source: Exclude<BinarySource, 'env' | 'package'>,
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
