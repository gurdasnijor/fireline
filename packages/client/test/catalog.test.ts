import { describe, expect, it } from 'vitest'

import { createCatalogClient, resolveAgentLaunch, type AgentCatalogEntry } from '../src/index.js'

describe('catalog', () => {
  it('merges local entries and resolves launchable registry agents', async () => {
    const client = createCatalogClient({
      fetchImpl: async () =>
        new Response(
          JSON.stringify({
            agents: [
              {
                id: 'codex-acp',
                name: 'Codex CLI',
                version: '1.2.3',
                distribution: {
                  npx: {
                    package: '@zed-industries/codex-acp@1.2.3',
                  },
                },
              },
              {
                id: 'cursor',
                name: 'Cursor',
                version: '9.9.9',
                distribution: {
                  binary: {
                    'darwin-aarch64': {
                      archive: 'https://example.com/cursor.tar.gz',
                      cmd: './cursor-agent',
                    },
                  },
                },
              },
            ],
          }),
          {
            status: 200,
            headers: {
              'content-type': 'application/json',
            },
          },
        ),
      localEntries: [
        {
          source: 'local',
          id: 'fireline-testy-load',
          name: 'Fireline Testy Load',
          version: 'local',
          distributions: [
            {
              kind: 'command',
              command: ['/tmp/fireline-testy-load'],
            },
          ],
        },
      ],
    })

    const agents = await client.listAgents()
    expect(agents.map((agent) => agent.id)).toEqual(['codex-acp', 'cursor', 'fireline-testy-load'])

    const codex = await client.resolveAgent('codex-acp')
    expect(codex.command).toEqual(['npx', '-y', '@zed-industries/codex-acp@1.2.3'])
    expect(codex.distributionKind).toBe('npx')

    const local = await client.resolveAgent('fireline-testy-load')
    expect(local.command).toEqual(['/tmp/fireline-testy-load'])
    expect(local.distributionKind).toBe('command')
  })

  it('reports binary-only agents as unresolved until local install exists', () => {
    const binaryOnly: AgentCatalogEntry = {
      source: 'registry',
      id: 'cursor',
      name: 'Cursor',
      version: '1.0.0',
      distributions: [
        {
          kind: 'binary',
          targets: [
            {
              target: 'darwin-aarch64',
              archive: 'https://example.com/cursor.tar.gz',
              cmd: './cursor-agent',
            },
          ],
        },
      ],
    }

    expect(() => resolveAgentLaunch(binaryOnly, { platform: 'darwin', arch: 'aarch64' })).toThrow(
      /binary archive/,
    )
  })
})
