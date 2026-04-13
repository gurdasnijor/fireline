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
  runHostedRepl,
  runReplCommand,
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

test('parseArgs returns repl-specific help topic', () => {
  const args = parseArgs(['repl', '--help'])
  assert.equal(args.command, 'help')
  assert.equal(args.helpFor, 'repl')
})

test('parseArgs returns deploy-specific help topic', () => {
  const args = parseArgs(['deploy', '--help'])
  assert.equal(args.command, 'help')
  assert.equal(args.helpFor, 'deploy')
})

test('parseArgs captures an optional repl session id', () => {
  const args = parseArgs(['repl', 'session-123'])
  assert.equal(args.command, 'repl')
  assert.equal(args.sessionId, 'session-123')
})

test('parseArgs accepts run --repl', () => {
  const args = parseArgs(['run', 'agent.ts', '--repl'])
  assert.equal(args.command, 'run')
  assert.equal(args.file, 'agent.ts')
  assert.equal(args.repl, true)
})

test('parseArgs accepts shorthand --repl', () => {
  const args = parseArgs(['agent.ts', '--repl'])
  assert.equal(args.command, 'run')
  assert.equal(args.file, 'agent.ts')
  assert.equal(args.repl, true)
})

test('parseArgs preserves agents passthrough arguments', () => {
  const args = parseArgs(['agents', 'add', 'pi-acp'])
  assert.equal(args.command, 'agents')
  assert.deepEqual(args.passthroughArgs, ['add', 'pi-acp'])
})

test('runReplCommand attaches to an existing session id', async () => {
  const args = parseArgs(['repl', 'session-123'])
  let sessionId: string | null | undefined

  const exitCode = await runReplCommand(args, async (options = {}) => {
    sessionId = options.sessionId
    return 0
  })

  assert.equal(exitCode, 0)
  assert.equal(sessionId, 'session-123')
})

test('runReplCommand rejects spec-like arguments with a run hint', async () => {
  const args = parseArgs(['repl', 'docs/demos/assets/agent.ts'])

  await assert.rejects(
    async () => {
      await runReplCommand(args, async () => 0)
    },
    /looks like a spec path, not a session id\. Did you mean: fireline run docs\/demos\/assets\/agent\.ts --repl \?/,
  )
})

test('runReplCommand disambiguates a missing local host', async () => {
  const args = parseArgs(['repl'])

  await assert.rejects(
    async () => {
      await runReplCommand(args, async () => {
        throw new Error('connect ECONNREFUSED 127.0.0.1:4440')
      })
    },
    /no fireline host running on :4440\. Start one: fireline run <spec>/,
  )
})

test('runHostedRepl attaches to the started host and prints the new session id', async () => {
  const args = parseArgs(['run', 'docs/demos/assets/agent.ts', '--repl'])
  const lines: string[] = []

  const exitCode = await runHostedRepl(
    {
      acp: { url: 'ws://127.0.0.1:4440/acp' },
      id: 'sandbox-123',
      state: { url: 'http://127.0.0.1:4440/state' },
    },
    args,
    async (options = {}) => {
      assert.equal(options.acpUrl, 'ws://127.0.0.1:4440/acp')
      assert.equal(options.runtimeId, 'sandbox-123')
      assert.equal(options.serverUrl, 'http://127.0.0.1:4440')
      assert.equal(options.stateStreamUrl, 'http://127.0.0.1:4440/state')
      await options.onSessionReady?.('session-123')
      return 0
    },
    {
      log: (...values: unknown[]) => {
        lines.push(values.join(' '))
      },
    },
  )

  assert.equal(exitCode, 0)
  assert.match(lines.join('\n'), /session:\s+session-123/)
})

test('runHostedRepl forwards an existing session id when auto-attaching', async () => {
  const args = parseArgs(['run', 'docs/demos/assets/agent.ts', '--repl'])
  let sessionId: string | null | undefined

  const exitCode = await runHostedRepl(
    {
      acp: { url: 'ws://127.0.0.1:4440/acp' },
      id: 'sandbox-123',
      state: { url: 'http://127.0.0.1:4440/state' },
    },
    args,
    async (options = {}) => {
      sessionId = options.sessionId
      return 0
    },
    console,
    { sessionId: 'session-existing' },
  )

  assert.equal(exitCode, 0)
  assert.equal(sessionId, 'session-existing')
})

test('parseArgs parses deploy target and native passthrough args', () => {
  const args = parseArgs([
    'deploy',
    'agent.ts',
    '--to',
    'fly',
    '--target',
    'production',
    '--token',
    'shh',
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
  assert.equal(args.targetName, 'production')
  assert.equal(args.token, 'shh')
  assert.equal(args.name, 'reviewer')
  assert.equal(args.stateStream, 'session-1')
  assert.deepEqual(args.passthroughArgs, ['--remote-only'])
})

test('parseArgs allows deploy without --to so hosted config can resolve the target later', () => {
  const args = parseArgs(['deploy', 'agent.ts'])
  assert.equal(args.command, 'deploy')
  assert.equal(args.file, 'agent.ts')
  assert.equal(args.to, null)
})

test('unwrapDefaultExport peels nested tsImport default wrappers', () => {
  const harness = fixtureSpec('demo')
  const wrapped = { __esModule: true, default: { __esModule: true, default: harness } }
  assert.equal(unwrapDefaultExport(wrapped), harness)
})

test('loadSpec accepts docs demo assets via the CLI loader', async () => {
  const spec = await loadSpec(resolve(REPO_ROOT, 'docs/demos/assets/agent.ts'))
  // The load-bearing contract is that the loader returns a runnable spec with
  // .start(); the `kind` discriminator is slated for removal (mono-d8x).
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

test('createDockerBuildPlan materializes the requested spec and wires ARG SPEC', async () => {
  const buildContext = await mkdtemp(join(tmpdir(), 'fireline-build-context-'))
  const plan = await createDockerBuildPlan({
    buildContext,
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
  assert.match(plan.buildArg, /^SPEC=\.tmp\/fireline-embedded-spec-/)
  assert.match(plan.embeddedSpecRelativePath, /^\.tmp\/fireline-embedded-spec-.*\/spec\.json$/)
  assert.deepEqual(JSON.parse(await readFile(plan.embeddedSpecPath, 'utf8')), {
    name: 'reviewer',
    sandbox: { provider: 'docker' },
    middleware: [],
    agent: { kind: 'process', argv: ['echo', 'hi'] },
  })
  assert.deepEqual(plan.args, [
    'build',
    '--file',
    '/repo/docker/fireline-host.Dockerfile',
    '--tag',
    'fireline-reviewer:latest',
    '--build-arg',
    plan.buildArg,
    buildContext,
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
  assert.equal(plan.deployImageRef, 'registry.fly.io/reviewer:latest')
  assert.deepEqual(plan.args, [
    'deploy',
    '--config',
    '/tmp/fly.toml',
    '--image',
    'registry.fly.io/reviewer:latest',
    '--remote-only',
  ])
  assert.deepEqual(
    plan.steps.map((step) => [step.command, ...step.args]),
    [
      ['flyctl', 'auth', 'docker'],
      ['docker', 'tag', 'fireline-reviewer:latest', 'registry.fly.io/reviewer:latest'],
      ['docker', 'push', 'registry.fly.io/reviewer:latest'],
      ['flyctl', 'deploy', '--config', '/tmp/fly.toml', '--image', 'registry.fly.io/reviewer:latest', '--remote-only'],
    ],
  )
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
  assert.equal(plan.steps.length, 1)
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
  assert.equal(plan.files.length, 1)
  const contents = await readFile(plan.filePath, 'utf8')
  assert.match(contents, /image = "registry\.fly\.io\/reviewer:latest"/)
  assert.match(contents, /path = "\/healthz"/)
})

test('createTargetScaffoldPlan writes wrangler plus a local image wrapper for cloudflare containers', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'fireline-cli-'))
  const plan = createTargetScaffoldPlan({
    target: 'cloudflare',
    cwd,
    appName: 'reviewer',
    imageTag: 'fireline-reviewer:latest',
  })

  await writeTargetScaffold(plan)

  assert.equal(plan.fileName, 'wrangler.toml')
  assert.equal(plan.files.length, 2)
  const wrangler = await readFile(plan.filePath, 'utf8')
  const wrapper = await readFile(join(cwd, 'Dockerfile.fireline'), 'utf8')
  assert.match(wrangler, /image = "\.\/Dockerfile\.fireline"/)
  assert.match(wrapper, /FROM fireline-reviewer:latest/)
})

test('createTargetScaffoldPlan writes a durable docker-compose quickstart manifest', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'fireline-cli-'))
  const plan = createTargetScaffoldPlan({
    target: 'docker-compose',
    cwd,
    appName: 'reviewer',
    imageTag: 'fireline-reviewer:latest',
  })

  await writeTargetScaffold(plan)

  const contents = await readFile(plan.filePath, 'utf8')
  assert.match(contents, /fireline-data:\/var\/lib\/fireline/)
  assert.match(contents, /http:\/\/127\.0\.0\.1:7474\/healthz/)
})

test('createTargetScaffoldPlan writes a pvc-backed k8s manifest with optional imagePullSecrets', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'fireline-cli-'))
  const plan = createTargetScaffoldPlan({
    target: 'k8s',
    cwd,
    appName: 'reviewer',
    imageTag: 'fireline-reviewer:latest',
    environment: {
      FIRELINE_DEPLOY_IMAGE: 'ghcr.io/acme/fireline-reviewer:latest',
      FIRELINE_K8S_IMAGE_PULL_SECRET: 'registry-creds',
      FIRELINE_K8S_STORAGE_CLASS: 'fast-ssd',
      FIRELINE_K8S_STORAGE_SIZE: '10Gi',
    },
  })

  await writeTargetScaffold(plan)

  const contents = await readFile(plan.filePath, 'utf8')
  assert.match(contents, /kind: PersistentVolumeClaim/)
  assert.match(contents, /storageClassName: fast-ssd/)
  assert.match(contents, /storage: 10Gi/)
  assert.match(contents, /image: ghcr\.io\/acme\/fireline-reviewer:latest/)
  assert.match(contents, /imagePullSecrets:/)
  assert.match(contents, /name: registry-creds/)
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
  const calls: Array<{ command: string; args: readonly string[]; cwd?: string; env?: NodeJS.ProcessEnv }> = []
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
    loadHostedConfig: async () => null,
    runChild: async (command, childArgs, options = {}) => {
      calls.push({ command, args: [...childArgs], cwd: options.cwd, env: options.env })
      return 0
    },
    writeTargetScaffold: async (plan) => {
      writtenPaths.push(plan.filePath)
    },
  })

  assert.equal(exitCode, 0)
  assert.equal(calls.length, 5)
  assert.equal(calls[0].command, 'docker')
  assert.deepEqual(calls[0].args.slice(0, 5), [
    'build',
    '--file',
    calls[0].args[2],
    '--tag',
    'fireline-reviewer:latest',
  ])
  assert.equal(calls[1].command, 'flyctl')
  assert.deepEqual(calls[1].args, ['auth', 'docker'])
  assert.equal(calls[2].command, 'docker')
  assert.deepEqual(calls[2].args, ['tag', 'fireline-reviewer:latest', 'registry.fly.io/reviewer:latest'])
  assert.equal(calls[3].command, 'docker')
  assert.deepEqual(calls[3].args, ['push', 'registry.fly.io/reviewer:latest'])
  assert.equal(calls[4].command, 'flyctl')
  assert.match(String(calls[4].args[2]), /fly\.toml$/)
  assert.deepEqual(calls[4].args.slice(-1), ['--remote-only'])
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
      loadHostedConfig: async () => null,
      runChild: async () => {
        callCount += 1
        if (callCount === 1) return 0
        throw new Error('failed to start flyctl: spawn flyctl ENOENT')
      },
      writeTargetScaffold: async () => {},
    }),
    /flyctl is required for "Authenticate docker against Fly registry"\.\nInstall flyctl \(`brew install flyctl`\): https:\/\/fly\.io\/docs\/flyctl\/install\//,
  )
})

test('createDeployExecutionPlan rejects k8s deploys without a pullable image reference', () => {
  assert.throws(
    () => createDeployExecutionPlan({
      target: 'k8s',
      cwd: '/repo',
      imageTag: 'fireline-reviewer:latest',
      scaffoldPath: '/tmp/k8s.yaml',
      passthroughArgs: ['--namespace', 'fireline'],
      environment: {},
    }),
    /deploy target 'k8s' requires a pullable image reference/,
  )
})

test('deploy resolves provider and token from hosted config when --to is omitted', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'fireline-cli-deploy-'))
  const calls: Array<{ command: string; args: readonly string[]; env?: NodeJS.ProcessEnv }> = []
  const args = parseArgs(['deploy', 'agent.ts', '--target', 'production'])
  const previousToken = process.env.FLY_API_TOKEN
  process.env.FLY_API_TOKEN = 'env-fly-token'

  try {
    const exitCode = await deploy(args, {
      cwd: () => cwd,
      loadSpec: async () => fixtureSpec('reviewer'),
      loadHostedConfig: async () => ({
        defaultTarget: 'production',
        targets: {
          production: {
            provider: 'fly',
            authRef: 'FLY_API_TOKEN',
            resourceNaming: { appName: 'reviewer-prod' },
          },
        },
      }),
      runChild: async (command, childArgs, options = {}) => {
        calls.push({ command, args: [...childArgs], env: options.env })
        return 0
      },
      writeTargetScaffold: async () => {},
    })

    assert.equal(exitCode, 0)
    // Fly multi-step plan: docker build, flyctl auth docker, docker tag,
    // docker push, flyctl deploy. Token must thread through every step.
    assert.equal(calls.length, 5)
    assert.equal(calls[4].command, 'flyctl')
    assert.equal(calls[4].env?.FLY_API_TOKEN, 'env-fly-token')
    assert.match(String(calls[0].args[4]), /fireline-reviewer-prod:latest/)
  } finally {
    if (previousToken === undefined) {
      delete process.env.FLY_API_TOKEN
    } else {
      process.env.FLY_API_TOKEN = previousToken
    }
  }
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
