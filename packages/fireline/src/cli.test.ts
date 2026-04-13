import assert from 'node:assert/strict'
import { mkdir, mkdtemp, readFile, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import {
  createDeployExecutionPlan,
  createDockerBuildPlan,
  createTargetScaffoldPlan,
  deploy,
  loadSpec,
  parseArgs,
  unwrapDefaultExport,
  writeTargetScaffold,
} from './cli.js'
import { findBinary } from './resolve-binary.js'

const TEST_FILE = fileURLToPath(import.meta.url)
const CLI_PACKAGE_DIR = resolve(dirname(TEST_FILE), '..')
const REPO_ROOT = resolve(CLI_PACKAGE_DIR, '..', '..')

test('parseArgs parses build flags', () => {
  const args = parseArgs([
    'build',
    'agent.ts',
    '--target',
    'fly',
    '--name',
    'reviewer',
    '--state-stream',
    'session-1',
    '--provider',
    'docker',
  ])

  assert.equal(args.command, 'build')
  assert.equal(args.helpFor, 'build')
  assert.equal(args.file, 'agent.ts')
  assert.equal(args.target, 'fly')
  assert.equal(args.name, 'reviewer')
  assert.equal(args.stateStream, 'session-1')
  assert.equal(args.providerOverride, 'docker')
})

test('parseArgs rejects run-only flags with build', () => {
  assert.throws(
    () => parseArgs(['build', 'agent.ts', '--port', '4440']),
    /--port is only valid with run/,
  )
})

test('parseArgs returns build-specific help topic', () => {
  const args = parseArgs(['build', '--help'])
  assert.equal(args.command, 'help')
  assert.equal(args.helpFor, 'build')
})

test('parseArgs returns agents-specific help topic', () => {
  const args = parseArgs(['agents', '--help'])
  assert.equal(args.command, 'help')
  assert.equal(args.helpFor, 'agents')
})

test('parseArgs returns deploy-specific help topic', () => {
  const args = parseArgs(['deploy', '--help'])
  assert.equal(args.command, 'help')
  assert.equal(args.helpFor, 'deploy')
})

test('parseArgs preserves agents passthrough arguments', () => {
  const args = parseArgs(['agents', 'add', 'pi-acp'])
  assert.equal(args.command, 'agents')
  assert.deepEqual(args.passthroughArgs, ['add', 'pi-acp'])
})

test('parseArgs parses deploy target and native passthrough args', () => {
  const args = parseArgs([
    'deploy',
    'agent.ts',
    '--to',
    'fly',
    '--name',
    'reviewer',
    '--state-stream',
    'session-1',
    '--',
    '--remote-only',
  ])

  assert.equal(args.command, 'deploy')
  assert.equal(args.helpFor, 'deploy')
  assert.equal(args.file, 'agent.ts')
  assert.equal(args.to, 'fly')
  assert.equal(args.name, 'reviewer')
  assert.equal(args.stateStream, 'session-1')
  assert.deepEqual(args.passthroughArgs, ['--remote-only'])
})

test('parseArgs rejects build-only flags with deploy', () => {
  assert.throws(
    () => parseArgs(['deploy', 'agent.ts', '--to', 'fly', '--target', 'k8s']),
    /--target is only valid with build/,
  )
})

test('parseArgs requires --to for deploy', () => {
  assert.throws(
    () => parseArgs(['deploy', 'agent.ts']),
    /deploy requires --to <platform>/,
  )
})

test('unwrapDefaultExport peels nested tsImport default wrappers', () => {
  const harness = fixtureSpec('demo')
  const wrapped = { __esModule: true, default: { __esModule: true, default: harness } }
  assert.equal(unwrapDefaultExport(wrapped), harness)
})

test('loadSpec accepts docs demo assets via the CLI loader', async () => {
  const spec = await loadSpec(resolve(REPO_ROOT, 'docs/demos/assets/agent.ts'))
  assert.equal(spec.kind, 'harness')
  assert.equal(typeof spec.start, 'function')
})

test('findBinary prefers target/release over target/debug', async () => {
  const root = await mkdtemp(join(tmpdir(), 'fireline-binary-'))
  await mkdir(join(root, 'target', 'release'), { recursive: true })
  await mkdir(join(root, 'target', 'debug'), { recursive: true })
  await writeFile(join(root, 'target', 'release', 'fireline'), '')
  await writeFile(join(root, 'target', 'debug', 'fireline'), '')

  const resolved = findBinary({
    name: 'fireline',
    envVar: 'FIRELINE_BIN',
    searchFrom: join(root, 'nested', 'dir'),
  })

  assert.equal(resolved?.source, 'release')
  assert.match(String(resolved?.path), /target\/release\/fireline$/)
})

test('findBinary falls back to target/debug when release is absent', async () => {
  const root = await mkdtemp(join(tmpdir(), 'fireline-binary-'))
  await mkdir(join(root, 'target', 'debug'), { recursive: true })
  await writeFile(join(root, 'target', 'debug', 'fireline-streams'), '')

  const resolved = findBinary({
    name: 'fireline-streams',
    envVar: 'FIRELINE_STREAMS_BIN',
    searchFrom: join(root, 'nested', 'dir'),
  })

  assert.equal(resolved?.source, 'debug')
  assert.match(String(resolved?.path), /target\/debug\/fireline-streams$/)
})

test('findBinary honors env overrides before target lookup', async () => {
  const root = await mkdtemp(join(tmpdir(), 'fireline-binary-'))
  const envBin = join(root, 'fireline')
  await writeFile(envBin, '')
  process.env.FIRELINE_BIN = envBin

  try {
    const resolved = findBinary({
      name: 'fireline',
      envVar: 'FIRELINE_BIN',
      searchFrom: join(root, 'nested', 'dir'),
    })
    assert.equal(resolved?.source, 'env')
    assert.equal(resolved?.path, envBin)
  } finally {
    delete process.env.FIRELINE_BIN
  }
})

test('createDockerBuildPlan wires embedded spec build arg', () => {
  const plan = createDockerBuildPlan({
    buildContext: '/repo',
    dockerfile: '/repo/docker/fireline-host.Dockerfile',
    imageTag: 'fireline-reviewer:latest',
    spec: {
      name: 'reviewer',
      sandbox: { provider: 'docker' },
      middleware: [],
      agent: { kind: 'process', argv: ['echo', 'hi'] },
    },
  })

  assert.equal(plan.command, 'docker')
  assert.equal(plan.dockerfile, '/repo/docker/fireline-host.Dockerfile')
  assert.equal(plan.imageTag, 'fireline-reviewer:latest')
  assert.match(plan.buildArg, /^FIRELINE_EMBEDDED_SPEC=\{/)
  assert.deepEqual(plan.args, [
    'build',
    '--file',
    '/repo/docker/fireline-host.Dockerfile',
    '--tag',
    'fireline-reviewer:latest',
    '--build-arg',
    plan.buildArg,
    '/repo',
  ])
})

test('createDeployExecutionPlan maps fly target to flyctl deploy', () => {
  const plan = createDeployExecutionPlan({
    target: 'fly',
    cwd: '/repo',
    imageTag: 'fireline-reviewer:latest',
    scaffoldPath: '/tmp/fly.toml',
    passthroughArgs: ['--remote-only'],
  })

  assert.equal(plan.command, 'flyctl')
  assert.deepEqual(plan.args, [
    'deploy',
    '--config',
    '/tmp/fly.toml',
    '--image',
    'fireline-reviewer:latest',
    '--remote-only',
  ])
})

test('createDeployExecutionPlan maps docker-compose target to docker compose', () => {
  const plan = createDeployExecutionPlan({
    target: 'docker-compose',
    cwd: '/repo',
    imageTag: 'fireline-reviewer:latest',
    scaffoldPath: '/tmp/docker-compose.yml',
    passthroughArgs: ['--wait'],
  })

  assert.equal(plan.command, 'docker')
  assert.deepEqual(plan.args, [
    'compose',
    '-f',
    '/tmp/docker-compose.yml',
    'up',
    '-d',
    '--wait',
  ])
})

test('writeTargetScaffold writes target config with image reference', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'fireline-cli-'))
  const plan = createTargetScaffoldPlan({
    target: 'fly',
    cwd,
    appName: 'reviewer',
    imageTag: 'fireline-reviewer:latest',
  })

  await writeTargetScaffold(plan)

  assert.equal(plan.fileName, 'fly.toml')
  const contents = await readFile(plan.filePath, 'utf8')
  assert.match(contents, /image = "fireline-reviewer:latest"/)
  assert.match(contents, /path = "\/healthz"/)
})

test('createTargetScaffoldPlan refuses to overwrite an existing target file', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'fireline-cli-'))
  await writeFile(join(cwd, 'k8s.yaml'), '# existing\n')

  assert.throws(
    () => createTargetScaffoldPlan({
      target: 'k8s',
      cwd,
      appName: 'reviewer',
      imageTag: 'fireline-reviewer:latest',
    }),
    /refusing to overwrite existing scaffold file/,
  )
})

test('deploy builds first, writes a manifest, then runs the native deploy command', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'fireline-cli-deploy-'))
  const calls: Array<{ command: string; args: readonly string[]; cwd?: string }> = []
  const writtenPaths: string[] = []
  const args = parseArgs([
    'deploy',
    'agent.ts',
    '--to',
    'fly',
    '--name',
    'reviewer',
    '--',
    '--remote-only',
  ])

  const exitCode = await deploy(args, {
    cwd: () => cwd,
    loadSpec: async () => fixtureSpec('reviewer'),
    runChild: async (command, childArgs, options = {}) => {
      calls.push({ command, args: [...childArgs], cwd: options.cwd })
      return 0
    },
    writeTargetScaffold: async (plan) => {
      writtenPaths.push(plan.filePath)
    },
  })

  assert.equal(exitCode, 0)
  assert.equal(calls.length, 2)
  assert.equal(calls[0].command, 'docker')
  assert.deepEqual(calls[0].args.slice(0, 5), [
    'build',
    '--file',
    calls[0].args[2],
    '--tag',
    'fireline-reviewer:latest',
  ])
  assert.equal(calls[1].command, 'flyctl')
  assert.match(String(calls[1].args[2]), /fly\.toml$/)
  assert.deepEqual(calls[1].args.slice(-1), ['--remote-only'])
  assert.equal(writtenPaths.length, 1)
  assert.match(writtenPaths[0], /fly\.toml$/)
})

test('deploy adds install guidance when the native CLI is missing', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'fireline-cli-deploy-'))
  const args = parseArgs(['deploy', 'agent.ts', '--to', 'fly'])
  let callCount = 0

  await assert.rejects(
    () => deploy(args, {
      cwd: () => cwd,
      loadSpec: async () => fixtureSpec('reviewer'),
      runChild: async () => {
        callCount += 1
        if (callCount === 1) return 0
        throw new Error('failed to start flyctl: spawn flyctl ENOENT')
      },
      writeTargetScaffold: async () => {},
    }),
    /Install flyctl and retry\.\nInstall flyctl: https:\/\/fly\.io\/docs\/flyctl\/install\//,
  )
})

function fixtureSpec(name: string) {
  return {
    kind: 'harness' as const,
    name,
    sandbox: { provider: 'docker' },
    middleware: [],
    agent: { kind: 'process', argv: ['echo', 'hi'] },
    start: async () => ({
      id: 'sandbox-1',
      acp: { url: 'ws://127.0.0.1:4440/acp' },
      state: { url: 'http://127.0.0.1:7474/v1/stream/state' },
    }),
  }
}
