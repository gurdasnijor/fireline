import { chmod, copyFile, mkdir } from 'node:fs/promises'
import { accessSync, constants } from 'node:fs'
import { join, resolve } from 'node:path'

const BINARIES = ['fireline', 'fireline-streams', 'fireline-agents']

function parseArgs(argv) {
  const out = {
    artifactDir: null,
    packageDir: null,
    exeSuffix: '',
  }

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--artifact-dir') {
      out.artifactDir = argv[++i] ?? null
      continue
    }
    if (arg === '--package-dir') {
      out.packageDir = argv[++i] ?? null
      continue
    }
    if (arg === '--exe-suffix') {
      out.exeSuffix = argv[++i] ?? ''
      continue
    }
    throw new Error(`unknown flag: ${arg}`)
  }

  if (!out.artifactDir || !out.packageDir) {
    throw new Error(
      'usage: node scripts/stage-fireline-platform-package.mjs --artifact-dir <dir> --package-dir <dir> [--exe-suffix <suffix>]',
    )
  }

  return out
}

async function main() {
  const args = parseArgs(process.argv.slice(2))
  const artifactDir = resolve(args.artifactDir)
  const packageDir = resolve(args.packageDir)
  const binDir = join(packageDir, 'bin')

  await mkdir(binDir, { recursive: true })

  for (const binary of BINARIES) {
    const source = join(artifactDir, `${binary}${args.exeSuffix}`)
    const destination = join(binDir, binary)
    accessSync(source, constants.R_OK)
    await copyFile(source, destination)
    await chmod(destination, 0o755)
  }
}

await main()
