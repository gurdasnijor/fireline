import { db, type SandboxDescriptor, type SandboxHandle } from '@fireline/client'

type ProbeDeps = {
  readonly healthCheck?: () => Promise<boolean>
  readonly listSandboxes?: () => Promise<SandboxDescriptor[]>
  readonly loadDb?: typeof db
}

export type ExistingHostProbeResult =
  | { readonly kind: 'not-fireline' }
  | { readonly kind: 'no-live-sandboxes' }
  | { readonly kind: 'multiple-live-sandboxes'; readonly count: number }
  | {
      readonly kind: 'attachable'
      readonly handle: SandboxHandle
      readonly latestSessionId: string | null
    }

export async function probeExistingHostForRepl(
  serverUrl: string,
  deps: ProbeDeps = {},
): Promise<ExistingHostProbeResult> {
  const healthCheck = deps.healthCheck ?? (() => probeHealth(serverUrl))
  const listSandboxes = deps.listSandboxes ?? (() => fetchSandboxes(serverUrl))

  if (!(await healthCheck().catch(() => false))) {
    return { kind: 'not-fireline' }
  }

  const sandboxes = await listSandboxes().catch(() => null)
  if (!sandboxes) {
    return { kind: 'not-fireline' }
  }

  const liveSandboxes = sandboxes
    .filter(
      (descriptor: SandboxDescriptor) =>
        descriptor.status !== 'stopped' && descriptor.status !== 'broken',
    )
    .sort(
      (left: SandboxDescriptor, right: SandboxDescriptor) =>
        right.updatedAtMs - left.updatedAtMs,
    )

  if (liveSandboxes.length === 0) {
    return { kind: 'no-live-sandboxes' }
  }

  if (liveSandboxes.length > 1) {
    return {
      kind: 'multiple-live-sandboxes',
      count: liveSandboxes.length,
    }
  }

  const descriptor = liveSandboxes[0]
  const latestSessionId = await probeLatestSessionId(
    descriptor,
    deps.loadDb ?? db,
  )

  return {
    kind: 'attachable',
    handle: {
      id: descriptor.id,
      provider: descriptor.provider,
      acp: descriptor.acp,
      state: descriptor.state,
    },
    latestSessionId,
  }
}

async function probeHealth(serverUrl: string): Promise<boolean> {
  const response = await fetch(`${serverUrl.replace(/\/$/, '')}/healthz`, {
    headers: { accept: 'text/plain' },
  })
  return response.ok
}

async function fetchSandboxes(serverUrl: string): Promise<SandboxDescriptor[]> {
  const response = await fetch(`${serverUrl.replace(/\/$/, '')}/v1/sandboxes`, {
    headers: { accept: 'application/json' },
  })
  if (!response.ok) {
    throw new Error(`list sandboxes returned ${response.status}`)
  }
  return (await response.json()) as SandboxDescriptor[]
}

async function probeLatestSessionId(
  descriptor: SandboxDescriptor,
  loadDb: typeof db,
): Promise<string | null> {
  const stateDb = await loadDb({ stateStreamUrl: descriptor.state.url })
  try {
    const sessions = [...stateDb.collections.sessions.toArray].sort(
      (left, right) =>
        right.lastSeenAt - left.lastSeenAt ||
        right.updatedAt - left.updatedAt ||
        right.createdAt - left.createdAt,
    )
    return sessions[0]?.sessionId ?? null
  } finally {
    stateDb.close()
  }
}
