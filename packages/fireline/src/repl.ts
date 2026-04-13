import {
  ClientSideConnection,
  PROTOCOL_VERSION,
  type Client,
  type InitializeResponse,
  type SessionNotification,
  type ToolCallContent,
  type ToolCallStatus,
  type UsageUpdate,
  type Stream,
} from '@agentclientprotocol/sdk'
import fireline, { appendApprovalResolved, type FirelineDB } from '@fireline/client'
import { SandboxAdmin } from '@fireline/client/admin'
import type {
  PermissionRow,
  PromptRequestRow,
  SessionRow,
} from '@fireline/state'
import { render } from 'ink'
import React from 'react'
import { once } from 'node:events'
import type { Readable, Writable } from 'node:stream'
import WebSocket, { type RawData } from 'ws'
import {
  createAcpEventAdapter,
  createControlEventBus,
  createDurableEventAdapter,
  EventStreamStore,
  type EventStreamViewModel,
} from './repl-pane-events.js'
import { logReplDebug } from './repl-debug.js'
import { FirelineReplApp } from './repl-ui.js'

const DEFAULT_SERVER_URL = process.env.FIRELINE_URL ?? 'http://127.0.0.1:4440'
const DEFAULT_CLIENT_NAME = '@fireline/cli'
const DEFAULT_CLIENT_VERSION = '0.0.1'

const DEFAULT_INITIALIZE_OPTIONS = {
  protocolVersion: PROTOCOL_VERSION,
  clientCapabilities: { fs: { readTextFile: false } },
  clientInfo: {
    name: DEFAULT_CLIENT_NAME,
    version: DEFAULT_CLIENT_VERSION,
  },
} satisfies Parameters<ClientSideConnection['initialize']>[0]

export type ReplRole = 'assistant' | 'thought' | 'user'

export interface UsageSnapshot {
  readonly used: number
  readonly size: number
  readonly cost: number | null
}

export interface MessageEntry {
  readonly id: number
  readonly kind: 'message'
  readonly role: ReplRole
  readonly text: string
}

export interface ToolEntry {
  readonly id: number
  readonly kind: 'tool'
  readonly toolCallId: string
  readonly title: string
  readonly status: ToolCallStatus
  readonly toolKind: string | null
  readonly detail: string | null
}

export interface PlanEntry {
  readonly id: number
  readonly kind: 'plan'
  readonly items: readonly string[]
}

export type TranscriptEntry = MessageEntry | ToolEntry | PlanEntry

export interface PendingApproval {
  readonly requestId: string | number
  readonly sessionId: string
  readonly reason: string | null
  readonly summary: string
  readonly toolCallId: string | null
}

export interface SessionTab {
  readonly activePrompts: number
  readonly attached: boolean
  readonly pendingApprovals: number
  readonly sessionId: string
  readonly state: string | null
}

export interface ReplViewState {
  readonly acpUrl: string
  readonly adminBusy: boolean
  readonly adminMessage: string | null
  readonly busy: boolean
  readonly db: FirelineDB | null
  readonly entries: readonly TranscriptEntry[]
  readonly pendingTools: number
  readonly pendingApproval: PendingApproval | null
  readonly resolvingApproval: boolean
  readonly runtimeId: string | null
  readonly runtimeStatus: string | null
  readonly selectedSessionId: string | null
  readonly serverUrl: string
  readonly sessionId: string | null
  readonly sessionTabs: readonly SessionTab[]
  readonly stateStreamUrl: string | null
  readonly supportsRuntimeRestart: boolean
  readonly supportsSessionAttach: boolean
  readonly usage: UsageSnapshot | null
}

export interface ReplViewModel {
  attachSelectedSession(): Promise<void>
  getSnapshot(): ReplViewState
  restartRuntime(): Promise<void>
  resolvePendingApproval(allow: boolean): Promise<void>
  selectNextSession(): void
  selectPreviousSession(): void
  stopRuntime(): Promise<void>
  subscribe(listener: () => void): () => void
  submit(text: string): Promise<'ignored' | 'quit' | 'sent'>
}

export interface ReplConnectOptions {
  readonly acpUrl: string
  readonly onSessionUpdate: (
    notification: SessionNotification,
  ) => void | Promise<void>
}

export interface ReplSessionConnection {
  newSession: ClientSideConnection['newSession']
  loadSession: ClientSideConnection['loadSession']
  unstable_resumeSession: ClientSideConnection['unstable_resumeSession']
  prompt: ClientSideConnection['prompt']
}

export interface ReplConnection {
  readonly connection: ReplSessionConnection
  readonly initializeResponse: InitializeResponse
  close(): Promise<void>
}

export interface ReplOptions {
  readonly acpUrl?: string
  readonly alternateScreen?: boolean
  readonly connect?: (options: ReplConnectOptions) => Promise<ReplConnection>
  readonly cwd?: string
  readonly error?: Writable
  readonly input?: Readable
  readonly onRuntimeRestart?: (runtimeId: string) => Promise<ReplRuntimeHandle>
  readonly onSessionReady?: (sessionId: string) => void | Promise<void>
  readonly output?: Writable
  readonly runtimeId?: string | null
  readonly serverUrl?: string
  readonly sessionId?: string | null
  readonly stateStreamUrl?: string | null
}

export interface ReplRuntimeHandle {
  readonly acp: { readonly url: string }
  readonly id: string
  readonly state: { readonly url: string }
}

export interface ReplSessionAttachResult {
  readonly mode: 'load' | 'noop' | 'resume'
}

export interface ReplRuntimeRestartResult {
  readonly acpUrl: string
  readonly runtimeId: string | null
  readonly serverUrl: string
  readonly sessionId: string
  readonly stateStreamUrl: string | null
}

export async function runRepl(options: ReplOptions = {}): Promise<number> {
  const input = options.input ?? process.stdin
  const output = options.output ?? process.stdout
  const error = options.error ?? process.stderr
  const cwd = options.cwd ?? invocationCwd()
  let serverUrl = options.serverUrl ?? DEFAULT_SERVER_URL
  let acpUrl = options.acpUrl ?? resolveAcpUrl(serverUrl)
  let stateStreamUrl = options.stateStreamUrl ?? process.env.FIRELINE_STREAM_URL ?? null
  let runtimeId = options.runtimeId ?? null
  logReplDebug('repl.start', {
    acpUrl,
    runtimeId,
    serverUrl,
    sessionId: options.sessionId ?? null,
    stateStreamUrl,
  })
  let activeSessionId = options.sessionId ?? null
  let controller: ReplController | null = null
  let db: FirelineDB | null = null
  let dbConsoleRestore: (() => void) | null = null
  let dbSubscriptions: Array<{ unsubscribe(): void }> = []
  let durableEvents: { close(): void } | null = null
  let latestSessions: SessionRow[] = []
  let latestPermissions: PermissionRow[] = []
  let latestPromptRequests: PromptRequestRow[] = []
  let repl: ReplConnection | null = null
  let replConnection: ReplSessionConnection | null = null
  let initializeResponse: InitializeResponse | null = null
  const eventStore = new EventStreamStore()
  const controlEvents = createControlEventBus(eventStore)

  const refreshSessionCatalog = () => {
    controller?.setSessionTabs(
      buildSessionTabs(
        latestSessions,
        latestPermissions,
        latestPromptRequests,
        controller?.getSnapshot().sessionId ?? activeSessionId,
      ),
    )
  }

  const refreshPendingApproval = () => {
    if (!controller) {
      return
    }
    const sessionId = controller.getSnapshot().sessionId
    controller.setPendingApproval(
      sessionId ? selectPendingApproval(latestPermissions, sessionId) : null,
    )
  }

  const closeStateSurface = async () => {
    durableEvents?.close()
    durableEvents = null
    for (const subscription of dbSubscriptions) {
      subscription.unsubscribe()
    }
    dbSubscriptions = []
    latestSessions = []
    latestPermissions = []
    latestPromptRequests = []
    controller?.setStateDb(null)
    controller?.setSessionTabs([])
    controller?.setPendingApproval(null)
    db?.close()
    db = null
    dbConsoleRestore?.()
    dbConsoleRestore = null
  }

  const attachStateSurface = async (nextStateStreamUrl: string | null) => {
    await closeStateSurface()
    if (!nextStateStreamUrl) {
      return
    }
    const restoreConsole = suppressStreamDbConsole()
    try {
      db = await fireline.db({ stateStreamUrl: nextStateStreamUrl })
      dbConsoleRestore = restoreConsole
      controller?.setStateDb(db)
      dbSubscriptions = [
        db.sessions.subscribe((rows: SessionRow[]) => {
          latestSessions = [...rows]
          refreshSessionCatalog()
        }),
        db.permissions.subscribe((rows: PermissionRow[]) => {
          latestPermissions = [...rows]
          refreshSessionCatalog()
          refreshPendingApproval()
        }),
        db.promptRequests.subscribe((rows: PromptRequestRow[]) => {
          latestPromptRequests = [...rows]
          refreshSessionCatalog()
        }),
      ]
      durableEvents = await createDurableEventAdapter({
        sink: eventStore,
        stateStreamUrl: nextStateStreamUrl,
      })
    } catch (error) {
      restoreConsole()
      throw error
    }
  }

  const closeReplConnection = async () => {
    const active = repl
    repl = null
    replConnection = null
    initializeResponse = null
    if (active) {
      await active.close()
    }
  }

  const openReplConnection = async (nextAcpUrl: string) => {
    repl = await (options.connect ?? connectReplAcp)({
      acpUrl: nextAcpUrl,
      onSessionUpdate: async (notification) => {
        eventStore.append({
          timestamp: Date.now(),
          source: 'acp',
          name: 'session_update',
          payload: 'dir=in notification',
          requestId:
            notification.update.sessionUpdate === 'usage_update'
              ? null
              : extractNotificationRequestId(notification),
          sessionId: notification.sessionId,
        })
        if (activeSessionId && notification.sessionId !== activeSessionId) {
          return
        }
        controller?.receiveNotification(notification)
      },
    })
    initializeResponse = repl.initializeResponse
    replConnection = createAcpEventAdapter({
      connection: repl.connection,
      sink: eventStore,
    }).connection as ReplSessionConnection
  }

  await openReplConnection(acpUrl)
  controller = new ReplController({
    acpUrl,
    runtimeId,
    attachSession: async (sessionId) => {
      if (!replConnection || !initializeResponse) {
        throw new Error('ACP session connection is not available')
      }
      if (sessionId === activeSessionId) {
        return { mode: 'noop' }
      }
      const mode = await attachExistingSession(
        replConnection,
        initializeResponse,
        sessionId,
        cwd,
      )
      activeSessionId = sessionId
      refreshPendingApproval()
      controlEvents.emit({
        name: 'session.attach',
        payload: `mode=${mode} session=${sessionId}`,
        sessionId,
      })
      return { mode }
    },
    sendPrompt: async (text) => {
      if (!replConnection || !activeSessionId) {
        throw new Error('ACP session connection is not available')
      }
      await replConnection.prompt({
        sessionId: activeSessionId!,
        prompt: [{ type: 'text', text }],
      })
    },
    resolveApproval: async (approval, allow) => {
      if (!stateStreamUrl) {
        throw new Error('state stream is required for approval resolution')
      }
      await appendApprovalResolved({
        allow,
        requestId: approval.requestId,
        resolvedBy: 'cli-repl',
        sessionId: approval.sessionId,
        streamUrl: stateStreamUrl,
      })
    },
    restartRuntime: options.onRuntimeRestart && runtimeId
      ? async () => {
          controlEvents.emit({
            name: 'runtime.restart.requested',
            payload: `runtime=${runtimeId}`,
            sessionId: activeSessionId,
          })
          await closeReplConnection()
          const nextHandle = await options.onRuntimeRestart!(runtimeId!)
          runtimeId = nextHandle.id
          acpUrl = nextHandle.acp.url
          stateStreamUrl = nextHandle.state.url
          await openReplConnection(acpUrl)
          const preferredSessionId =
            controller?.getSnapshot().selectedSessionId ?? activeSessionId
          try {
            activeSessionId = preferredSessionId
              ? await ensureSession(replConnection!, initializeResponse!, preferredSessionId, cwd)
              : await ensureSession(replConnection!, initializeResponse!, null, cwd)
          } catch {
            activeSessionId = await ensureSession(
              replConnection!,
              initializeResponse!,
              null,
              cwd,
            )
          }
          await attachStateSurface(stateStreamUrl)
          refreshPendingApproval()
          controlEvents.emit({
            name: 'runtime.restarted',
            payload: `runtime=${runtimeId}`,
            sessionId: activeSessionId,
          })
          return {
            acpUrl,
            runtimeId,
            serverUrl,
            sessionId: activeSessionId,
            stateStreamUrl,
          }
        }
      : undefined,
    serverUrl,
    sessionId: options.sessionId ?? null,
    stateStreamUrl,
    stopRuntime: runtimeId
      ? async () => {
          controlEvents.emit({
            name: 'runtime.stop.requested',
            payload: `runtime=${runtimeId}`,
            sessionId: activeSessionId,
          })
          const admin = new SandboxAdmin({ serverUrl })
          await admin.destroy(runtimeId!)
          await closeReplConnection()
          controlEvents.emit({
            name: 'runtime.stopped',
            payload: `runtime=${runtimeId}`,
            sessionId: activeSessionId,
            severity: 'warning',
          })
        }
      : undefined,
  })

  try {
    activeSessionId = await ensureSession(
      replConnection!,
      initializeResponse!,
      activeSessionId,
      cwd,
    )

    await options.onSessionReady?.(activeSessionId)
    controller.setSessionId(activeSessionId)
    controller.setRuntimeStatus(runtimeId ? await readRuntimeStatus(serverUrl, runtimeId) : null)
    await attachStateSurface(stateStreamUrl)
    refreshPendingApproval()
    return await runInkRepl({
      alternateScreen: options.alternateScreen ?? true,
      controller,
      events: eventStore,
      error: error as NodeJS.WriteStream,
      input: input as NodeJS.ReadStream,
      output: output as NodeJS.WriteStream,
    })
  } finally {
    await closeStateSurface()
    await closeReplConnection()
  }
}

export async function connectReplAcp(
  options: ReplConnectOptions,
): Promise<ReplConnection> {
  const socket = new WebSocket(options.acpUrl)
  await waitForSocketOpen(socket)

  const connection = new ClientSideConnection(
    () => createClientHandler(options.onSessionUpdate),
    createWebSocketStream(socket),
  )
  const initializeResponse = await connection.initialize(
    DEFAULT_INITIALIZE_OPTIONS,
  )

  return {
    connection,
    initializeResponse,
    close: async () => {
      await closeSocket(socket)
    },
  }
}

export function resolveAcpUrl(serverUrl: string): string {
  const url = new URL(serverUrl)
  if (url.protocol === 'http:') {
    url.protocol = 'ws:'
  } else if (url.protocol === 'https:') {
    url.protocol = 'wss:'
  }
  const pathname = url.pathname === '/' ? '' : url.pathname.replace(/\/$/, '')
  url.pathname = `${pathname}/acp`
  url.search = ''
  url.hash = ''
  return url.toString()
}

async function runInkRepl(options: {
  readonly alternateScreen: boolean
  readonly controller: ReplController
  readonly events: EventStreamViewModel
  readonly error: NodeJS.WriteStream
  readonly input: NodeJS.ReadStream
  readonly output: NodeJS.WriteStream
}): Promise<number> {
  let exitCode = 0
  let failure: Error | null = null
  const app = render(
    React.createElement(FirelineReplApp, {
      controller: options.controller,
      events: options.events,
      onExitRequest: (code) => {
        exitCode = code
      },
      onFailure: (error) => {
        failure = error
      },
    }),
    {
      alternateScreen: options.alternateScreen,
      exitOnCtrlC: false,
      incrementalRendering: true,
      patchConsole: false,
      stderr: options.error,
      stdin: options.input,
      stdout: options.output,
    },
  )

  try {
    await app.waitUntilExit()
    if (failure) {
      throw failure
    }
    return exitCode
  } catch (error) {
    throw error instanceof Error ? error : new Error(String(error))
  } finally {
    app.cleanup()
  }
}

async function ensureSession(
  connection: ReplSessionConnection,
  initializeResponse: InitializeResponse,
  sessionId: string | null,
  cwd: string,
): Promise<string> {
  if (!sessionId) {
    const session = await connection.newSession({ cwd, mcpServers: [] })
    return session.sessionId
  }

  const capabilities = initializeResponse.agentCapabilities
  if (capabilities?.sessionCapabilities?.resume) {
    await connection.unstable_resumeSession({ cwd, sessionId, mcpServers: [] })
    return sessionId
  }
  if (capabilities?.loadSession) {
    await connection.loadSession({ cwd, sessionId, mcpServers: [] })
    return sessionId
  }

  throw new Error(
    `session '${sessionId}' was provided, but the agent does not advertise session resume or load support`,
  )
}

async function attachExistingSession(
  connection: ReplSessionConnection,
  initializeResponse: InitializeResponse,
  sessionId: string,
  cwd: string,
): Promise<'load' | 'resume'> {
  const capabilities = initializeResponse.agentCapabilities
  if (capabilities?.sessionCapabilities?.resume) {
    await connection.unstable_resumeSession({ cwd, sessionId, mcpServers: [] })
    return 'resume'
  }
  if (capabilities?.loadSession) {
    await connection.loadSession({ cwd, sessionId, mcpServers: [] })
    return 'load'
  }
  throw new Error(
    `session '${sessionId}' cannot be attached because the agent does not advertise session resume or load support`,
  )
}

async function readRuntimeStatus(
  serverUrl: string,
  runtimeId: string,
): Promise<string | null> {
  try {
    const admin = new SandboxAdmin({ serverUrl })
    return await admin.status(runtimeId)
  } catch {
    return null
  }
}

function extractNotificationRequestId(
  notification: SessionNotification,
): string | number | null {
  const update = notification.update as Record<string, unknown>
  if (typeof update.requestId === 'string' || typeof update.requestId === 'number') {
    return update.requestId
  }
  if (typeof update.id === 'string' || typeof update.id === 'number') {
    return update.id
  }
  return null
}

function createClientHandler(
  onSessionUpdate: ReplConnectOptions['onSessionUpdate'],
): Client {
  return {
    requestPermission: async () => ({ outcome: { outcome: 'cancelled' } }),
    sessionUpdate: async (notification) => {
      await onSessionUpdate(notification)
    },
    writeTextFile: async () => unsupported('writeTextFile'),
    readTextFile: async () => unsupported('readTextFile'),
    createTerminal: async () => unsupported('createTerminal'),
    terminalOutput: async () => unsupported('terminalOutput'),
    releaseTerminal: async () => unsupported('releaseTerminal'),
    waitForTerminalExit: async () => unsupported('waitForTerminalExit'),
    killTerminal: async () => unsupported('killTerminal'),
    extMethod: async (method: string) => unsupported(`extMethod:${method}`),
    extNotification: async () => {},
  }
}

function unsupported(name: string): never {
  throw new Error(
    `ACP SDK requires stub client method '${name}'. Build on @agentclientprotocol/sdk directly for custom client behavior.`,
  )
}

function createWebSocketStream(socket: WebSocket): Stream {
  return {
    readable: new ReadableStream({
      start(controller) {
        const handleMessage = (data: RawData) => {
          controller.enqueue(JSON.parse(decodeRawData(data)))
        }
        const handleClose = () => {
          cleanup()
          controller.close()
        }
        const handleError = (error: Error) => {
          cleanup()
          controller.error(error)
        }
        const cleanup = () => {
          socket.off('message', handleMessage)
          socket.off('close', handleClose)
          socket.off('error', handleError)
        }

        socket.on('message', handleMessage)
        socket.on('close', handleClose)
        socket.on('error', handleError)
      },
    }),
    writable: new WritableStream({
      write(message) {
        return new Promise<void>((resolve, reject) => {
          socket.send(JSON.stringify(message), (error: Error | undefined) => {
            if (error) {
              reject(error)
              return
            }
            resolve()
          })
        })
      },
      close() {
        socket.close()
      },
      abort() {
        socket.close()
      },
    }),
  }
}

async function waitForSocketOpen(socket: WebSocket): Promise<void> {
  if (socket.readyState === WebSocket.OPEN) {
    return
  }

  await new Promise<void>((resolve, reject) => {
    const onOpen = () => {
      cleanup()
      resolve()
    }
    const onError = (error: Error) => {
      cleanup()
      reject(error)
    }
    const cleanup = () => {
      socket.off('open', onOpen)
      socket.off('error', onError)
    }

    socket.once('open', onOpen)
    socket.once('error', onError)
  })
}

async function closeSocket(socket: WebSocket): Promise<void> {
  if (
    socket.readyState === WebSocket.CLOSED ||
    socket.readyState === WebSocket.CLOSING
  ) {
    return
  }

  const closed = once(socket, 'close').then(() => {})
  socket.close()
  await closed
}

function decodeRawData(data: RawData): string {
  if (typeof data === 'string') {
    return data
  }

  if (Buffer.isBuffer(data)) {
    return data.toString('utf8')
  }

  if (Array.isArray(data)) {
    return Buffer.concat(data).toString('utf8')
  }

  if (data instanceof ArrayBuffer) {
    return Buffer.from(data).toString('utf8')
  }

  return Buffer.concat(data).toString('utf8')
}

function invocationCwd(): string {
  return process.env.PWD || process.cwd()
}

interface ApprovalWatcher {
  close(): Promise<void>
}

async function watchApprovals(
  stateStreamUrl: string,
  sessionId: string,
  onPendingApproval: (approval: PendingApproval | null) => void,
): Promise<ApprovalWatcher> {
  const restoreConsole = suppressStreamDbConsole()
  try {
    const db = await fireline.db({ stateStreamUrl })
    const subscription = db.permissions.subscribe((rows: PendingApprovalRow[]) => {
      onPendingApproval(selectPendingApproval(rows, sessionId))
    })

    return {
      close: async () => {
        try {
          subscription.unsubscribe()
          db.close()
        } finally {
          restoreConsole()
        }
      },
    }
  }
  catch (error) {
    restoreConsole()
    throw error
  }
}

function selectPendingApproval(
  rows: readonly PendingApprovalRow[],
  sessionId: string,
): PendingApproval | null {
  const pending = rows
    .filter(
      (row) =>
        row.sessionId === sessionId &&
        row.state === 'pending' &&
        isApprovalRequestId(row.requestId),
    )
    .sort((left, right) => left.createdAt - right.createdAt)[0]
  if (!pending) {
    return null
  }

  const requestId = pending.requestId
  if (!isApprovalRequestId(requestId)) {
    return null
  }

  return {
    requestId,
    reason: pending.title ?? null,
    sessionId: pending.sessionId,
    summary: pending.title ?? pending.toolCallId ?? `request ${String(requestId)}`,
    toolCallId: pending.toolCallId ?? null,
  }
}

function isApprovalRequestId(value: unknown): value is string | number {
  return typeof value === 'string' || typeof value === 'number'
}

interface PendingApprovalRow {
  readonly createdAt: number
  readonly requestId?: string | number | null
  readonly sessionId: string
  readonly state: string
  readonly title?: string
  readonly toolCallId?: string
}

function suppressStreamDbConsole(): () => void {
  const methods = ['log', 'warn', 'error'] as const
  const originals = methods.map((name) => [name, console[name].bind(console)] as const)

  for (const [name, original] of originals) {
    console[name] = ((...args: unknown[]) => {
      if (typeof args[0] === 'string' && args[0].startsWith('[StreamDB]')) {
        return
      }
      original(...args)
    }) as typeof console.log
  }

  return () => {
    for (const [name, original] of originals) {
      console[name] = original
    }
  }
}

export class ReplController implements ReplViewModel {
  private readonly listeners = new Set<() => void>()
  private readonly toolIndexes = new Map<string, number>()
  private readonly toolStatuses = new Map<string, ToolCallStatus>()
  private nextEntryId = 1
  private currentChunkEntryId: number | null = null
  private state: ReplViewState

  constructor(
    private readonly options: {
      readonly acpUrl?: string
      readonly attachSession?: (
        sessionId: string,
      ) => Promise<ReplSessionAttachResult>
      readonly restartRuntime?: () => Promise<ReplRuntimeRestartResult>
      readonly runtimeId?: string | null
      readonly sendPrompt: (text: string) => Promise<void>
      readonly resolveApproval: (
        approval: PendingApproval,
        allow: boolean,
      ) => Promise<void>
      readonly serverUrl: string
      readonly sessionId: string | null
      readonly stateStreamUrl?: string | null
      readonly stopRuntime?: () => Promise<void>
    },
  ) {
    this.state = {
      acpUrl: options.acpUrl ?? resolveAcpUrl(options.serverUrl),
      adminBusy: false,
      adminMessage: null,
      busy: false,
      db: null,
      entries: [],
      pendingTools: 0,
      pendingApproval: null,
      resolvingApproval: false,
      runtimeId: options.runtimeId ?? null,
      runtimeStatus: null,
      selectedSessionId: options.sessionId,
      serverUrl: options.serverUrl,
      sessionId: options.sessionId,
      sessionTabs: options.sessionId
        ? [{
            activePrompts: 0,
            attached: true,
            pendingApprovals: 0,
            sessionId: options.sessionId,
            state: null,
          }]
        : [],
      stateStreamUrl: options.stateStreamUrl ?? null,
      supportsRuntimeRestart: Boolean(options.restartRuntime),
      supportsSessionAttach: Boolean(options.attachSession),
      usage: null,
    }
  }

  getSnapshot(): ReplViewState {
    return this.state
  }

  subscribe(listener: () => void): () => void {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }

  selectNextSession(): void {
    this.shiftSelectedSession(1)
  }

  selectPreviousSession(): void {
    this.shiftSelectedSession(-1)
  }

  async attachSelectedSession(): Promise<void> {
    const selectedSessionId = this.state.selectedSessionId
    if (!selectedSessionId) {
      this.setAdminMessage('No session is selected.')
      return
    }
    if (!this.options.attachSession) {
      this.setAdminMessage('Session attach is unavailable for this REPL attachment.')
      return
    }
    if (selectedSessionId === this.state.sessionId) {
      this.setAdminMessage(`Already attached to ${selectedSessionId}.`)
      return
    }

    this.setAdminBusy(true)
    try {
      const result = await this.options.attachSession(selectedSessionId)
      this.replaceAttachedSession(
        selectedSessionId,
        result.mode === 'noop'
          ? `Already attached to ${selectedSessionId}.`
          : `${result.mode === 'resume' ? 'Resumed' : 'Loaded'} ${selectedSessionId}.`,
      )
    } catch (error) {
      this.setAdminBusy(false)
      this.setAdminMessage(formatAdminError('Session attach failed', error))
      throw error
    }
  }

  async stopRuntime(): Promise<void> {
    if (!this.options.stopRuntime || !this.state.runtimeId) {
      this.setAdminMessage('Runtime stop is unavailable for this REPL attachment.')
      return
    }

    this.setAdminBusy(true)
    try {
      await this.options.stopRuntime()
      this.state = {
        ...this.state,
        adminBusy: false,
        adminMessage: `Stopped runtime ${this.state.runtimeId}.`,
        runtimeStatus: 'stopped',
      }
      this.emit()
    } catch (error) {
      this.setAdminBusy(false)
      this.setAdminMessage(formatAdminError('Runtime stop failed', error))
      throw error
    }
  }

  async restartRuntime(): Promise<void> {
    if (!this.options.restartRuntime) {
      this.setAdminMessage('Runtime restart is unavailable for this REPL attachment.')
      return
    }

    this.setAdminBusy(true)
    try {
      const result = await this.options.restartRuntime()
      this.state = {
        ...this.state,
        acpUrl: result.acpUrl,
        adminBusy: false,
        adminMessage: `Restarted runtime ${result.runtimeId ?? 'unknown'} and reattached ${result.sessionId}.`,
        busy: false,
        entries: [],
        pendingApproval: null,
        pendingTools: 0,
        resolvingApproval: false,
        runtimeId: result.runtimeId,
        runtimeStatus: 'ready',
        selectedSessionId: result.sessionId,
        serverUrl: result.serverUrl,
        sessionId: result.sessionId,
        stateStreamUrl: result.stateStreamUrl,
        usage: null,
      }
      this.toolIndexes.clear()
      this.toolStatuses.clear()
      this.currentChunkEntryId = null
      this.appendMessage('thought', `Restarted runtime and reattached ${result.sessionId}.`)
      this.emit()
    } catch (error) {
      this.setAdminBusy(false)
      this.setAdminMessage(formatAdminError('Runtime restart failed', error))
      throw error
    }
  }

  async resolvePendingApproval(allow: boolean): Promise<void> {
    const approval = this.state.pendingApproval
    if (!approval || this.state.resolvingApproval) {
      logReplDebug('approval.resolve.skipped', {
        allow,
        hasPendingApproval: Boolean(approval),
        resolvingApproval: this.state.resolvingApproval,
      })
      return
    }

    logReplDebug('approval.resolve.begin', {
      action: allow ? 'allow' : 'deny',
      requestId: approval.requestId,
      sessionId: approval.sessionId,
      toolCallId: approval.toolCallId,
    })
    this.state = { ...this.state, resolvingApproval: true }
    this.emit()

    try {
      await this.options.resolveApproval(approval, allow)
      this.state = {
        ...this.state,
        pendingApproval: null,
        resolvingApproval: false,
      }
      logReplDebug('approval.resolve.success', {
        action: allow ? 'allow' : 'deny',
        requestId: approval.requestId,
      })
      this.emit()
    } catch (error) {
      this.state = { ...this.state, resolvingApproval: false }
      logReplDebug('approval.resolve.error', {
        action: allow ? 'allow' : 'deny',
        error,
        requestId: approval.requestId,
      })
      this.emit()
      throw error
    }
  }

  setSessionId(sessionId: string): void {
    this.state = {
      ...this.state,
      selectedSessionId: sessionId,
      sessionId,
      sessionTabs: mergeAttachedTab(this.state.sessionTabs, sessionId),
    }
    this.emit()
  }

  setSessionTabs(sessionTabs: readonly SessionTab[]): void {
    const selectedSessionId =
      this.state.selectedSessionId &&
      sessionTabs.some((tab) => tab.sessionId === this.state.selectedSessionId)
        ? this.state.selectedSessionId
        : this.state.sessionId &&
            sessionTabs.some((tab) => tab.sessionId === this.state.sessionId)
          ? this.state.sessionId
          : sessionTabs[0]?.sessionId ?? this.state.selectedSessionId
    this.state = {
      ...this.state,
      selectedSessionId,
      sessionTabs: mergeAttachedTabs(sessionTabs, this.state.sessionId),
    }
    this.emit()
  }

  setRuntimeStatus(runtimeStatus: string | null): void {
    this.state = { ...this.state, runtimeStatus }
    this.emit()
  }

  setStateDb(db: FirelineDB | null): void {
    this.state = { ...this.state, db }
    this.emit()
  }

  setPendingApproval(approval: PendingApproval | null): void {
    logReplDebug('approval.pending.set', {
      reason: approval?.reason ?? null,
      requestId: approval?.requestId ?? null,
      summary: approval?.summary ?? null,
      toolCallId: approval?.toolCallId ?? null,
    })
    this.state = {
      ...this.state,
      pendingApproval: approval,
      resolvingApproval: approval ? this.state.resolvingApproval : false,
    }
    this.emit()
  }

  async submit(text: string): Promise<'ignored' | 'quit' | 'sent'> {
    const trimmed = text.trim()
    if (!trimmed) {
      return 'ignored'
    }
    if (trimmed === '/quit') {
      return 'quit'
    }
    if (
      this.state.selectedSessionId &&
      this.state.sessionId &&
      this.state.selectedSessionId !== this.state.sessionId
    ) {
      this.setAdminMessage(
        `Selected session ${this.state.selectedSessionId} is not attached. Press l to load or resume it first.`,
      )
      return 'ignored'
    }

    this.appendMessage('user', text)
    this.setBusy(true)
    try {
      await this.options.sendPrompt(text)
    } finally {
      this.currentChunkEntryId = null
      this.setBusy(false)
    }

    return 'sent'
  }

  private shiftSelectedSession(delta: -1 | 1): void {
    if (this.state.sessionTabs.length === 0) {
      return
    }
    const currentIndex = Math.max(
      0,
      this.state.sessionTabs.findIndex(
        (tab) => tab.sessionId === this.state.selectedSessionId,
      ),
    )
    const nextIndex =
      (currentIndex + delta + this.state.sessionTabs.length) %
      this.state.sessionTabs.length
    this.state = {
      ...this.state,
      adminMessage: `Selected ${this.state.sessionTabs[nextIndex]!.sessionId}.`,
      selectedSessionId: this.state.sessionTabs[nextIndex]!.sessionId,
    }
    this.emit()
  }

  private replaceAttachedSession(sessionId: string, message: string): void {
    this.toolIndexes.clear()
    this.toolStatuses.clear()
    this.currentChunkEntryId = null
    this.state = {
      ...this.state,
      adminBusy: false,
      adminMessage: message,
      busy: false,
      entries: [],
      pendingApproval: null,
      pendingTools: 0,
      resolvingApproval: false,
      selectedSessionId: sessionId,
      sessionId,
      sessionTabs: mergeAttachedTab(this.state.sessionTabs, sessionId),
      usage: null,
    }
    this.appendMessage('thought', message)
  }

  private setAdminBusy(adminBusy: boolean): void {
    this.state = { ...this.state, adminBusy }
    this.emit()
  }

  private setAdminMessage(adminMessage: string | null): void {
    this.state = { ...this.state, adminMessage }
    this.emit()
  }

  receiveNotification(notification: SessionNotification): void {
    const update = notification.update
    switch (update.sessionUpdate) {
      case 'agent_message_chunk':
        this.appendChunk('assistant', update.content)
        return
      case 'agent_thought_chunk':
        this.appendChunk('thought', update.content)
        return
      case 'user_message_chunk':
        this.appendChunk('user', update.content)
        return
      case 'tool_call':
        this.upsertTool(update.toolCallId, {
          detail: summarizeToolDetails(update.content),
          status: update.status ?? 'pending',
          title: update.title,
          toolKind: update.kind ?? null,
        })
        return
      case 'tool_call_update':
        this.upsertTool(update.toolCallId, {
          detail: summarizeToolDetails(update.content ?? undefined),
          status: update.status ?? undefined,
          title: update.title ?? undefined,
          toolKind: update.kind ?? undefined,
        })
        return
      case 'plan':
        this.appendPlan(update.entries.map((entry) => `${entry.status} - ${entry.content}`))
        return
      case 'usage_update':
        this.setUsage(update)
        return
      default:
        return
    }
  }

  private appendChunk(role: ReplRole, content: { type: string; text?: string }): void {
    if (content.type !== 'text' || !content.text) {
      return
    }

    const lastEntry = this.state.entries[this.state.entries.length - 1]
    if (
      lastEntry?.kind === 'message' &&
      lastEntry.id === this.currentChunkEntryId &&
      lastEntry.role === role
    ) {
      const entries = [...this.state.entries]
      const index = entries.length - 1
      entries[index] = {
        ...lastEntry,
        text: `${lastEntry.text}${content.text}`,
      }
      this.state = { ...this.state, entries }
      this.emit()
      return
    }

    this.appendMessage(role, content.text)
  }

  private appendMessage(role: ReplRole, text: string): void {
    const entry: MessageEntry = {
      id: this.nextEntryId++,
      kind: 'message',
      role,
      text,
    }
    this.currentChunkEntryId = entry.id
    this.state = {
      ...this.state,
      entries: [...this.state.entries, entry],
    }
    this.emit()
  }

  private appendPlan(items: readonly string[]): void {
    if (items.length === 0) {
      return
    }

    this.currentChunkEntryId = null
    const entry: PlanEntry = {
      id: this.nextEntryId++,
      items,
      kind: 'plan',
    }
    this.state = {
      ...this.state,
      entries: [...this.state.entries, entry],
    }
    this.emit()
  }

  private setBusy(busy: boolean): void {
    this.state = { ...this.state, busy }
    this.emit()
  }

  private setUsage(update: UsageUpdate): void {
    this.state = {
      ...this.state,
      usage: {
        cost: typeof update.cost?.amount === 'number' ? update.cost.amount : null,
        size: update.size,
        used: update.used,
      },
    }
    this.emit()
  }

  private upsertTool(
    toolCallId: string,
    patch: {
      readonly detail?: string | null
      readonly status?: ToolCallStatus
      readonly title?: string
      readonly toolKind?: string | null
    },
  ): void {
    this.currentChunkEntryId = null
    const existingIndex = this.toolIndexes.get(toolCallId)
    const existing =
      existingIndex === undefined ? null : this.state.entries[existingIndex]

    if (existing?.kind === 'tool' && existingIndex !== undefined) {
      const nextEntry: ToolEntry = {
        ...existing,
        detail: patch.detail ?? existing.detail,
        status: patch.status ?? existing.status,
        title: patch.title ?? existing.title,
        toolKind:
          patch.toolKind === undefined ? existing.toolKind : patch.toolKind,
      }
      const entries = [...this.state.entries]
      entries[existingIndex] = nextEntry
      this.toolStatuses.set(toolCallId, nextEntry.status)
      this.state = {
        ...this.state,
        entries,
        pendingTools: countPendingTools(this.toolStatuses),
      }
      this.emit()
      return
    }

    const entry: ToolEntry = {
      id: this.nextEntryId++,
      kind: 'tool',
      toolCallId,
      title: patch.title ?? `Tool ${toolCallId}`,
      status: patch.status ?? 'pending',
      toolKind: patch.toolKind ?? null,
      detail: patch.detail ?? null,
    }
    const entries = [...this.state.entries, entry]
    this.toolIndexes.set(toolCallId, entries.length - 1)
    this.toolStatuses.set(toolCallId, entry.status)
    this.state = {
      ...this.state,
      entries,
      pendingTools: countPendingTools(this.toolStatuses),
    }
    this.emit()
  }

  private emit(): void {
    for (const listener of this.listeners) {
      listener()
    }
  }
}

function buildSessionTabs(
  sessions: readonly SessionRow[],
  permissions: readonly PermissionRow[],
  promptRequests: readonly PromptRequestRow[],
  attachedSessionId: string | null,
): SessionTab[] {
  const approvalCounts = new Map<string, number>()
  for (const permission of permissions) {
    if (permission.state !== 'pending') {
      continue
    }
    approvalCounts.set(
      permission.sessionId,
      (approvalCounts.get(permission.sessionId) ?? 0) + 1,
    )
  }

  const activePromptCounts = new Map<string, number>()
  for (const promptRequest of promptRequests) {
    if (
      promptRequest.state !== 'active' &&
      promptRequest.state !== 'queued' &&
      promptRequest.state !== 'cancel_requested'
    ) {
      continue
    }
    activePromptCounts.set(
      promptRequest.sessionId,
      (activePromptCounts.get(promptRequest.sessionId) ?? 0) + 1,
    )
  }

  const tabs = sessions
    .slice()
    .sort((left, right) => (right.lastSeenAt ?? right.updatedAt) - (left.lastSeenAt ?? left.updatedAt))
    .map((row) => ({
      activePrompts: activePromptCounts.get(row.sessionId) ?? 0,
      attached: row.sessionId === attachedSessionId,
      pendingApprovals: approvalCounts.get(row.sessionId) ?? 0,
      sessionId: row.sessionId,
      state: row.state,
    }))

  return mergeAttachedTabs(tabs, attachedSessionId)
}

function mergeAttachedTabs(
  tabs: readonly SessionTab[],
  attachedSessionId: string | null,
): SessionTab[] {
  if (!attachedSessionId) {
    return [...tabs]
  }
  const existing = tabs.find((tab) => tab.sessionId === attachedSessionId)
  if (!existing) {
    return [
      {
        activePrompts: 0,
        attached: true,
        pendingApprovals: 0,
        sessionId: attachedSessionId,
        state: null,
      },
      ...tabs,
    ]
  }
  return tabs.map((tab) =>
    tab.sessionId === attachedSessionId ? { ...tab, attached: true } : { ...tab, attached: false },
  )
}

function mergeAttachedTab(
  tabs: readonly SessionTab[],
  attachedSessionId: string | null,
): SessionTab[] {
  return mergeAttachedTabs(tabs, attachedSessionId)
}

function formatAdminError(prefix: string, error: unknown): string {
  const message = error instanceof Error ? error.message : String(error)
  return `${prefix}: ${message}`
}

function countPendingTools(
  toolStatuses: ReadonlyMap<string, ToolCallStatus>,
): number {
  let pending = 0
  for (const status of toolStatuses.values()) {
    if (status === 'pending' || status === 'in_progress') {
      pending++
    }
  }
  return pending
}

function summarizeToolDetails(
  content: readonly ToolCallContent[] | null | undefined,
): string | null {
  if (!content || content.length === 0) {
    return null
  }

  for (const item of content) {
    if (item.type === 'content' && item.content.type === 'text') {
      return clamp(item.content.text)
    }
    if (item.type === 'diff') {
      return 'diff update'
    }
    if (item.type === 'terminal') {
      return clamp(`terminal ${item.terminalId}`)
    }
  }

  return null
}

function clamp(text: string): string {
  const normalized = text.replace(/\s+/g, ' ').trim()
  if (normalized.length <= 120) {
    return normalized
  }
  return `${normalized.slice(0, 117)}...`
}
