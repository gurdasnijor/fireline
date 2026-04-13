import assert from 'node:assert/strict'
import { chmod, mkdir, rm, writeFile } from 'node:fs/promises'
import { createRequire } from 'node:module'
import { dirname, join, resolve as resolvePath } from 'node:path'
import { tmpdir } from 'node:os'
import test from 'node:test'
import { fileURLToPath } from 'node:url'
import { resolveBinary } from './resolve-binary.js'

const PLATFORM_PACKAGES: Record<string, string> = {
  'darwin-arm64': '@fireline/cli-darwin-arm64',
  'darwin-x64': '@fireline/cli-darwin-x64',
  'linux-arm64': '@fireline/cli-linux-arm64',
  'linux-x64': '@fireline/cli-linux-x64',
  'win32-x64': '@fireline/cli-win32-x64',
}

const packageRoot = resolvePath(dirname(fileURLToPath(import.meta.url)), '..')

test(
  'resolveBinary honors FIRELINE_* overrides before package lookup',
  { concurrency: false },
  async () => {
    const overrideDir = await mkdirTempDir('fireline-bin-override-')
    const overridePath = join(overrideDir, 'fireline')
    await writeExecutable(overridePath, '#!/bin/sh\nexit 0\n')

    const previous = process.env.FIRELINE_BIN
    process.env.FIRELINE_BIN = overridePath

    try {
      const resolved = resolveBinary({ name: 'fireline', envVar: 'FIRELINE_BIN' })
      assert.equal(resolved.path, overridePath)
      assert.equal(resolved.source, 'env')
    } finally {
      restoreEnv('FIRELINE_BIN', previous)
      await rm(overrideDir, { recursive: true, force: true })
    }
  },
)

test(
  'resolveBinary resolves binaries from the current platform package',
  { concurrency: false },
  async (t) => {
    const platformKey = `${process.platform}-${process.arch}`
    const platformPackage = PLATFORM_PACKAGES[platformKey]
    if (!platformPackage) {
      t.skip(`no platform package mapping for ${platformKey}`)
      return
    }

    const require_ = createRequire(import.meta.url)
    const pkgJsonPath = require_.resolve(`${platformPackage}/package.json`)
    const binDir = join(dirname(pkgJsonPath), 'bin')

    await mkdir(binDir, { recursive: true })

    const writtenPaths = await Promise.all([
      writeExecutable(join(binDir, 'fireline'), '#!/bin/sh\nexit 0\n'),
      writeExecutable(join(binDir, 'fireline-streams'), '#!/bin/sh\nexit 0\n'),
      writeExecutable(join(binDir, 'fireline-agents'), '#!/bin/sh\nexit 0\n'),
    ])

    t.after(async () => {
      await Promise.all(writtenPaths.map((path) => rm(path, { force: true })))
    })

    const fireline = resolveBinary({ name: 'fireline', envVar: 'FIRELINE_BIN' })
    assert.equal(fireline.path, join(binDir, 'fireline'))
    assert.equal(fireline.source, 'package')

    const streams = resolveBinary({
      name: 'fireline-streams',
      envVar: 'FIRELINE_STREAMS_BIN',
    })
    assert.equal(streams.path, join(binDir, 'fireline-streams'))
    assert.equal(streams.source, 'package')

    const agents = resolveBinary({
      name: 'fireline-agents',
      envVar: 'FIRELINE_AGENTS_BIN',
    })
    assert.equal(agents.path, join(binDir, 'fireline-agents'))
    assert.equal(agents.source, 'package')
  },
)

test(
  'resolveBinary falls back to target/debug when the platform package is missing',
  { concurrency: false },
  async () => {
    const debugDir = join(packageRoot, 'target', 'debug')
    const debugBinary = join(debugDir, 'fireline-streams')
    await mkdir(debugDir, { recursive: true })
    await writeExecutable(debugBinary, '#!/bin/sh\nexit 0\n')

    try {
      const resolved = resolveBinary({
        name: 'fireline-streams',
        envVar: 'FIRELINE_STREAMS_BIN',
        searchFrom: join(packageRoot, 'nested', 'dir'),
      })
      assert.equal(resolved.path, debugBinary)
      assert.equal(resolved.source, 'debug')
    } finally {
      await rm(join(packageRoot, 'target'), { recursive: true, force: true })
    }
  },
)

test(
  'resolveBinary prefers target binaries under process.cwd() before packaged binaries',
  { concurrency: false },
  async () => {
    const workspaceRoot = await mkdirTempDir('fireline-workspace-')
    const nestedDir = join(workspaceRoot, 'apps', 'demo')
    const releaseDir = join(workspaceRoot, 'target', 'release')
    const releaseBinary = join(releaseDir, 'fireline')
    const previousCwd = process.cwd()

    await mkdir(nestedDir, { recursive: true })
    await mkdir(releaseDir, { recursive: true })
    await writeExecutable(releaseBinary, '#!/bin/sh\nexit 0\n')
    process.chdir(nestedDir)

    try {
      const resolved = resolveBinary({ name: 'fireline', envVar: 'FIRELINE_BIN' })
      assert.equal(resolved.path, releaseBinary)
      assert.equal(resolved.source, 'release')
    } finally {
      process.chdir(previousCwd)
      await rm(workspaceRoot, { recursive: true, force: true })
    }
  },
)

async function mkdirTempDir(prefix: string): Promise<string> {
  const { mkdtemp } = await import('node:fs/promises')
  return mkdtemp(join(tmpdir(), prefix))
}

async function writeExecutable(path: string, contents: string): Promise<string> {
  await writeFile(path, contents)
  await chmod(path, 0o755)
  return path
}

function restoreEnv(name: string, value: string | undefined) {
  if (value === undefined) {
    delete process.env[name]
    return
  }
  process.env[name] = value
}
