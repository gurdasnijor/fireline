import { existsSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

/**
 * Binary lookup strategy (esbuild/turbo pattern):
 *   1. Environment variable override (FIRELINE_BIN / FIRELINE_STREAMS_BIN)
 *   2. Platform-specific optional dep (stubbed for now)
 *   3. `target/debug/<bin>` relative to the workspace root (dev fallback)
 */

const PLATFORM_PACKAGES: Record<string, string> = {
  'darwin-arm64': '@fireline/cli-darwin-arm64',
  'darwin-x64': '@fireline/cli-darwin-x64',
  'linux-arm64': '@fireline/cli-linux-arm64',
  'linux-x64': '@fireline/cli-linux-x64',
  'win32-x64': '@fireline/cli-win32-x64',
}

export interface BinaryLookup {
  /** Name of the executable. */
  readonly name: 'fireline' | 'fireline-streams'
  /** Env var that overrides the lookup. */
  readonly envVar: string
}

export function resolveBinary(lookup: BinaryLookup): string {
  const envOverride = process.env[lookup.envVar]
  if (envOverride && existsSync(envOverride)) return envOverride
  if (envOverride) {
    throw new Error(
      `${lookup.envVar}=${envOverride} but no binary exists at that path`,
    )
  }

  // Platform package (lookup only — package may not be installed)
  const platformKey = `${process.platform}-${process.arch}`
  const platformPkg = PLATFORM_PACKAGES[platformKey]
  if (platformPkg) {
    try {
      const require_ = createRequire(import.meta.url)
      const pkgJsonPath = require_.resolve(`${platformPkg}/package.json`)
      const binPath = join(dirname(pkgJsonPath), 'bin', lookup.name)
      if (existsSync(binPath)) return binPath
    } catch {
      // platform package not installed; fall through to dev fallback
    }
  }

  // Dev fallback: find target/debug/<name> walking up from this file
  const devPath = findDevBinary(lookup.name)
  if (devPath) return devPath

  throw new Error(
    [
      `Could not find '${lookup.name}' binary.`,
      `Tried:`,
      `  - $${lookup.envVar} (not set or file does not exist)`,
      `  - ${platformPkg ?? `no platform package for ${platformKey}`} (not installed)`,
      `  - target/debug/${lookup.name} (not found walking up from ${import.meta.url})`,
      ``,
      `Fix: run 'cargo build --bin ${lookup.name}' from the fireline workspace root,`,
      `or set ${lookup.envVar} to an absolute path.`,
    ].join('\n'),
  )
}

function findDevBinary(name: string): string | null {
  let dir = dirname(fileURLToPath(import.meta.url))
  for (let i = 0; i < 10; i++) {
    const candidate = resolve(dir, 'target', 'debug', name)
    if (existsSync(candidate)) return candidate
    const release = resolve(dir, 'target', 'release', name)
    if (existsSync(release)) return release
    const parent = dirname(dir)
    if (parent === dir) break
    dir = parent
  }
  return null
}
