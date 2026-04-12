import assert from 'node:assert/strict'
import { mkdtemp, readFile, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'
import {
  createDockerBuildPlan,
  createTargetScaffoldPlan,
  parseArgs,
  writeTargetScaffold,
} from './cli.js'

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

test('parseArgs preserves agents passthrough arguments', () => {
  const args = parseArgs(['agents', 'add', 'pi-acp'])
  assert.equal(args.command, 'agents')
  assert.deepEqual(args.passthroughArgs, ['add', 'pi-acp'])
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
