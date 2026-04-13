import assert from 'node:assert/strict'
import { mkdir, mkdtemp, realpath, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'

import {
  buildDirectHostArgs,
  prepareMountedResources,
} from './fireline-embedded-spec-bootstrap.ts'

test('prepareMountedResources resolves embedded localPath resources relative to the mounted spec', async () => {
  const root = await mkdtemp(join(tmpdir(), 'fireline-embedded-spec-'))
  const specDir = join(root, 'spec-override')
  const workspaceDir = join(specDir, 'workspace')
  await mkdir(workspaceDir, { recursive: true })
  await writeFile(join(specDir, 'spec.json'), '{}\n')
  await writeFile(join(workspaceDir, 'README.md'), 'hello\n')
  const expectedWorkspaceDir = await realpath(workspaceDir)

  const mountedResources = await prepareMountedResources(
    {
      name: 'demo',
      agentCommand: ['echo', 'ok'],
      topology: null,
      resources: [
        {
          source_ref: { kind: 'localPath', path: './workspace' },
          mount_path: '/workspace',
        },
      ],
    },
    join(specDir, 'spec.json'),
  )

  assert.deepEqual(mountedResources, [
      {
        host_path: expectedWorkspaceDir,
        mount_path: '/workspace',
        read_only: false,
      },
  ])
})

test('buildDirectHostArgs forwards mounted resources to direct-host boot', () => {
  process.env.FIRELINE_DURABLE_STREAMS_URL = 'http://127.0.0.1:17474/v1/stream'
  delete process.env.FIRELINE_ADVERTISED_STATE_STREAM_URL
  try {
    const args = buildDirectHostArgs(
      {
        name: 'demo',
        agentCommand: ['echo', 'ok'],
        topology: { components: [] },
        stateStream: 'demo-stream',
      },
      [
        {
          host_path: '/tmp/spec-override/workspace',
          mount_path: '/workspace',
          read_only: false,
        },
      ],
    )

    assert.deepEqual(args, [
      '--host', '0.0.0.0',
      '--port', '4440',
      '--name', 'demo',
      '--durable-streams-url', 'http://127.0.0.1:17474/v1/stream',
      '--state-stream', 'demo-stream',
      '--advertised-state-stream-url', 'http://127.0.0.1:17474/v1/stream/demo-stream',
      '--mounted-resources-json', '[{"host_path":"/tmp/spec-override/workspace","mount_path":"/workspace","read_only":false}]',
      '--', 'echo', 'ok',
    ])
  } finally {
    delete process.env.FIRELINE_DURABLE_STREAMS_URL
  }
})

test('prepareMountedResources still rejects unsupported embedded resource kinds', async () => {
  const root = await mkdtemp(join(tmpdir(), 'fireline-embedded-spec-'))
  const specPath = join(root, 'spec.json')
  await writeFile(specPath, '{}\n')

  await assert.rejects(
    async () => {
      await prepareMountedResources(
        {
          name: 'demo',
          agentCommand: ['echo', 'ok'],
          topology: null,
          resources: [
            {
              source_ref: { kind: 'durableStreamBlob' },
              mount_path: '/workspace/blob.txt',
            },
          ],
        },
        specPath,
      )
    },
    /embedded-spec boot does not support resource kind 'durableStreamBlob'/,
  )
})
