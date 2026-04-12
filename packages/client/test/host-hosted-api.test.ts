import { afterAll, beforeAll, describe, expect, it } from 'vitest'

import { createHostedApiHost, type HostedApiHostOptions } from '../src/host-hosted-api/index.js'

type DummyHostedAgent = {
  readonly url: string
  stop(): Promise<void>
}

let fixture: DummyHostedAgent | undefined

describe('host-hosted-api', () => {
  beforeAll(async () => {
    const mod = await import('./fixtures/dummy-hosted-acp-agent.mjs')
    fixture = await mod.start()
  })

  afterAll(async () => {
    await fixture?.stop()
  })

  it('provisions, wakes, and stops a hosted runtime through the Host interface', async () => {
    const host = createHostedApiHost({
      endpointUrl: fixture!.url,
    } satisfies HostedApiHostOptions)

    const handle = await host.provision({
      model: 'dummy-hosted-model',
      metadata: {
        name: 'dummy-hosted-api-test',
      },
    })

    expect(handle.kind).toBe('hosted-api')
    expect(handle.id).toMatch(/\S+/)
    expect(handle.acp.url).toMatch(/\/acp$/)
    expect(handle.state.url).toMatch(/\/state$/)
    expect(await host.status(handle)).toEqual({ kind: 'running' })
    expect(await host.wake(handle)).toEqual({ kind: 'noop' })

    await host.stop(handle)

    expect(await host.status(handle)).toEqual({ kind: 'stopped' })
  })
})
