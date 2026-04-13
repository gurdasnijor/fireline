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
import fireline, { appendApprovalResolved } from '@fireline/client'
import { render } from 'ink'
import React from 'react'
import { once } from 'node:events'
import type { Readable, Writable } from 'node:stream'
import WebSocket, { type RawData } from 'ws'
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
  readonly summary: string
}

export interface ReplViewState {
  readonly busy: boolean
  readonly entries: readonly TranscriptEntry[]
  readonly pendingTools: number
  readonly pendingApproval: PendingApproval | null
  readonly resolvingApproval: boolean
  readonly serverUrl: string
  readonly sessionId: string | null
  readonly usage: UsageSnapshot | null
}

export interface ReplViewModel {
  getSnapshot(): ReplViewState
  resolvePendingApproval(allow: boolean): Promise<void>
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
  readonly onSessionReady?: (sessionId: string) => void | Promise<void>
  readonly output?: Writable
  readonly serverUrl?: string
  readonly sessionId?: string | null
  readonly stateStreamUrl?: string | null
}

export async function runRepl(options: ReplOptions = {}): Promise<number> {
  const input = options.input ?? process.stdin
  const output = options.output ?? process.stdout
  const error = options.error ?? process.stderr
  const cwd = options.cwd ?? invocationCwd()
  const serverUrl = options.serverUrl ?? DEFAULT_SERVER_URL
  const acpUrl = options.acpUrl ?? resolveAcpUrl(serverUrl)
  const stateStreamUrl = options.stateStreamUrl ?? process.env.FIRELINE_STREAM_URL ?? null
  let activeSessionId = options.sessionId ?? null
  let controller: ReplController | null = null
  let approvalWatcher: ApprovalWatcher | null = null
  const repl = await (options.connect ?? connectReplAcp)({
    acpUrl,
    onSessionUpdate: async (notification) => {
      if (activeSessionId && notification.sessionId !== activeSessionId) {
        return
      }
      controller?.receiveNotification(notification)
    },
  })
  controller = new ReplController({
    sendPrompt: async (text) => {
      await repl.connection.prompt({
        sessionId: activeSessionId!,
        prompt: [{ type: 'text', text }],
      })
    },
    resolveApproval: async (approval, allow) => {
      await appendApprovalResolved({
        allow,
        requestId: approval.requestId,
        resolvedBy: 'cli-repl',
        sessionId: approval.sessionId,
        streamUrl: stateStreamUrl!,
      })
    },
    serverUrl,
    sessionId: options.sessionId ?? null,
  })

  try {
    activeSessionId = await ensureSession(
      repl.connection,
      repl.initializeResponse,
      activeSessionId,
      cwd,
    )

    await options.onSessionReady?.(activeSessionId)
    controller.setSessionId(activeSessionId)
    if (stateStreamUrl) {
      approvalWatcher = await watchApprovals(stateStreamUrl, activeSessionId, (approval) => {
        controller?.setPendingApproval(approval)
      })
    }
    return await runInkRepl({
      alternateScreen: options.alternateScreen ?? true,
      controller,
      error: error as NodeJS.WriteStream,
      input: input as NodeJS.ReadStream,
      output: output as NodeJS.WriteStream,
    })
  } finally {
    await approvalWatcher?.close()
    await repl.close()
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
  readonly error: NodeJS.WriteStream
  readonly input: NodeJS.ReadStream
  readonly output: NodeJS.WriteStream
}): Promise<number> {
  let exitCode = 0
  let failure: Error | null = null
  const app = render(
    React.createElement(FirelineReplApp, {
      controller: options.controller,
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
    sessionId: pending.sessionId,
    summary: pending.title ?? pending.toolCallId ?? `request ${String(requestId)}`,
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
      readonly sendPrompt: (text: string) => Promise<void>
      readonly resolveApproval: (
        approval: PendingApproval,
        allow: boolean,
      ) => Promise<void>
      readonly serverUrl: string
      readonly sessionId: string | null
    },
  ) {
    this.state = {
      busy: false,
      entries: [],
      pendingTools: 0,
      pendingApproval: null,
      resolvingApproval: false,
      serverUrl: options.serverUrl,
      sessionId: options.sessionId,
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

  async resolvePendingApproval(allow: boolean): Promise<void> {
    const approval = this.state.pendingApproval
    if (!approval || this.state.resolvingApproval) {
      return
    }

    this.state = { ...this.state, resolvingApproval: true }
    this.emit()

    try {
      await this.options.resolveApproval(approval, allow)
      this.state = {
        ...this.state,
        pendingApproval: null,
        resolvingApproval: false,
      }
      this.emit()
    } catch (error) {
      this.state = { ...this.state, resolvingApproval: false }
      this.emit()
      throw error
    }
  }

  setSessionId(sessionId: string): void {
    this.state = { ...this.state, sessionId }
    this.emit()
  }

  setPendingApproval(approval: PendingApproval | null): void {
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
