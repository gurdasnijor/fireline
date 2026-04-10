import { mkdir, readFile } from 'node:fs/promises'
import { homedir, platform } from 'node:os'
import { dirname, join } from 'node:path'

import { parse } from '@iarna/toml'
import type { PromptResponse, SessionNotification, StopReason } from '@agentclientprotocol/sdk'

import { connectAcp, type AcpInitializeOptions } from './acp.js'

export interface PeerDescriptor {
  runtimeId: string
  agentName: string
  acpUrl: string
  stateStreamUrl?: string
  registeredAtMs: number
}

export interface PeerClientOptions {
  peerDirectoryPath?: string
}

export interface PeerParentLineage {
  traceId?: string
  parentSessionId?: string
  parentPromptTurnId?: string
}

export interface PeerCallRequest {
  agentName: string
  prompt: string
  /**
   * Working directory to send with `session/new`. Defaults to the current
   * process `cwd`. ACP requires an absolute path.
   */
  cwd?: string
  /**
   * Optional lineage metadata stamped onto the outgoing `initialize`
   * request under `_meta.fireline`, mirroring the Rust peer transport so
   * cross-runtime calls show up in trace stitching.
   */
  parentLineage?: PeerParentLineage
  /**
   * Millisecond budget for the full prompt turn (initialize → newSession →
   * prompt → stop reason). Defaults to 20 seconds.
   */
  timeoutMs?: number
}

export interface PeerCallResult {
  runtimeId: string
  agentName: string
  sessionId: string
  responseText: string
  stopReason: StopReason
  promptResponse: PromptResponse
}

export interface PeerClient {
  list(): Promise<PeerDescriptor[]>
  lookup(agentName: string): Promise<PeerDescriptor | null>
  call(request: PeerCallRequest): Promise<PeerCallResult>
}

interface PeerDirectoryFile {
  peers?: Array<Record<string, unknown>>
}

const DEFAULT_CALL_TIMEOUT_MS = 20_000

export function createPeerClient(options: PeerClientOptions = {}): PeerClient {
  const peerDirectoryPath = options.peerDirectoryPath ?? defaultPeerDirectoryPath()

  const peerClient: PeerClient = {
    async list() {
      return readPeerDirectory(peerDirectoryPath)
    },

    async lookup(agentName: string) {
      const peers = await readPeerDirectory(peerDirectoryPath)
      return peers.find((peer) => peer.agentName === agentName) ?? null
    },

    async call(request: PeerCallRequest): Promise<PeerCallResult> {
      const peer = await peerClient.lookup(request.agentName)
      if (!peer) {
        throw new Error(`peer '${request.agentName}' not found in ${peerDirectoryPath}`)
      }

      const timeoutMs = request.timeoutMs ?? DEFAULT_CALL_TIMEOUT_MS
      const deadline = Date.now() + timeoutMs

      const acp = await connectAcp({ url: peer.acpUrl })
      try {
        await acp.initialize(buildInitializeOptions(request.parentLineage))

        const session = await acp.connection.newSession({
          cwd: request.cwd ?? process.cwd(),
          mcpServers: [],
        })

        const updates = acp.updates()[Symbol.asyncIterator]()
        let responseText = ''

        const collector = (async () => {
          while (Date.now() < deadline) {
            const remaining = deadline - Date.now()
            const next = await Promise.race<
              IteratorResult<SessionNotification> | typeof COLLECTOR_TIMEOUT
            >([iteratorNext(updates), sleep(remaining).then(() => COLLECTOR_TIMEOUT)])

            if (next === COLLECTOR_TIMEOUT) {
              return
            }
            if (next.done) {
              return
            }
            const notification = next.value
            if (notification.sessionId !== session.sessionId) {
              continue
            }
            if (notification.update.sessionUpdate === 'agent_message_chunk') {
              const content = notification.update.content
              if (content.type === 'text') {
                responseText += content.text
              }
            }
          }
        })()

        const promptResponse = await acp.connection.prompt({
          sessionId: session.sessionId,
          prompt: [
            {
              type: 'text',
              text: request.prompt,
            },
          ],
        })

        // Give the collector a small window to drain any final chunks that
        // arrived right before the prompt response resolved.
        await Promise.race([collector, sleep(50)])

        return {
          runtimeId: peer.runtimeId,
          agentName: peer.agentName,
          sessionId: session.sessionId,
          responseText,
          stopReason: promptResponse.stopReason,
          promptResponse,
        }
      } finally {
        await acp.close()
      }
    },
  }

  return peerClient
}

export function defaultPeerDirectoryPath(): string {
  const home = homedir()
  switch (platform()) {
    case 'darwin':
      return join(home, 'Library', 'Application Support', 'fireline', 'peers.toml')
    case 'win32':
      return join(
        process.env.LOCALAPPDATA ?? join(home, 'AppData', 'Local'),
        'fireline',
        'peers.toml',
      )
    default:
      return join(
        process.env.XDG_DATA_HOME ?? join(home, '.local', 'share'),
        'fireline',
        'peers.toml',
      )
  }
}

async function readPeerDirectory(peerDirectoryPath: string): Promise<PeerDescriptor[]> {
  try {
    await mkdir(dirname(peerDirectoryPath), { recursive: true })
    const raw = await readFile(peerDirectoryPath, 'utf8')
    if (!raw.trim()) {
      return []
    }
    const parsed = parse(raw) as PeerDirectoryFile
    if (!Array.isArray(parsed.peers)) {
      return []
    }
    return parsed.peers.map(toPeerDescriptor).filter((peer): peer is PeerDescriptor => peer !== null)
  } catch (error) {
    if (isMissingFileError(error)) {
      return []
    }
    throw error
  }
}

function toPeerDescriptor(entry: Record<string, unknown>): PeerDescriptor | null {
  const runtimeId = readString(entry, 'runtime_id') ?? readString(entry, 'runtimeId')
  const agentName = readString(entry, 'agent_name') ?? readString(entry, 'agentName')
  const acpUrl = readString(entry, 'acp_url') ?? readString(entry, 'acpUrl')
  const registeredAtMs =
    readNumber(entry, 'registered_at_ms') ?? readNumber(entry, 'registeredAtMs') ?? 0

  if (!runtimeId || !agentName || !acpUrl) {
    return null
  }

  const stateStreamUrl =
    readString(entry, 'state_stream_url') ?? readString(entry, 'stateStreamUrl')

  return {
    runtimeId,
    agentName,
    acpUrl,
    stateStreamUrl,
    registeredAtMs,
  }
}

function buildInitializeOptions(
  parentLineage?: PeerParentLineage,
): AcpInitializeOptions | undefined {
  if (!parentLineage) {
    return undefined
  }
  const fireline: Record<string, string> = {}
  if (parentLineage.traceId) {
    fireline.traceId = parentLineage.traceId
  }
  if (parentLineage.parentSessionId) {
    fireline.parentSessionId = parentLineage.parentSessionId
  }
  if (parentLineage.parentPromptTurnId) {
    fireline.parentPromptTurnId = parentLineage.parentPromptTurnId
  }
  if (Object.keys(fireline).length === 0) {
    return undefined
  }
  return {
    meta: { fireline },
  }
}

function readString(entry: Record<string, unknown>, key: string): string | undefined {
  const value = entry[key]
  return typeof value === 'string' ? value : undefined
}

function readNumber(entry: Record<string, unknown>, key: string): number | undefined {
  const value = entry[key]
  return typeof value === 'number' ? value : undefined
}

function isMissingFileError(error: unknown): boolean {
  return (
    typeof error === 'object' &&
    error !== null &&
    'code' in error &&
    (error as { code?: string }).code === 'ENOENT'
  )
}

const COLLECTOR_TIMEOUT = Symbol('peer-call-collector-timeout')

async function iteratorNext(
  iterator: AsyncIterator<SessionNotification>,
): Promise<IteratorResult<SessionNotification>> {
  return iterator.next()
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, ms)
  })
}
