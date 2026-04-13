import type {
  CancelNotification,
  LoadSessionRequest,
  NewSessionRequest,
  PromptRequest,
  ResumeSessionRequest,
  SessionNotification,
} from '@agentclientprotocol/sdk'
import { stream, type JsonBatch } from '@durable-streams/client'
import { Box, Spacer, Text, useInput } from 'ink'
import React, { useMemo, useState, useSyncExternalStore } from 'react'
import { REPL_PALETTE } from './repl-palette.js'

const DEFAULT_MAX_EVENTS = 512
const DEFAULT_VISIBLE_ROWS = 10

export type EventSourceTag = 'acp' | 'durable' | 'control'
export type EventSeverity = 'info' | 'warning' | 'error'

export interface RealtimeEvent {
  readonly id: string
  readonly timestamp: number
  readonly source: EventSourceTag
  readonly name: string
  readonly payload: string
  readonly sessionId?: string | null
  readonly requestId?: string | number | null
  readonly severity?: EventSeverity
}

export interface EventStreamSnapshot {
  readonly controlSurfaceNote: string | null
  readonly events: readonly RealtimeEvent[]
}

export interface EventStreamViewModel {
  getSnapshot(): EventStreamSnapshot
  subscribe(listener: () => void): () => void
}

export interface EventStreamSink {
  append(event: Omit<RealtimeEvent, 'id'> | RealtimeEvent): void
  noteControlSurfaceGap(note: string): void
}

type StoredRealtimeEvent = RealtimeEvent & { readonly sequence: number }

export class EventStreamStore implements EventStreamViewModel, EventStreamSink {
  private readonly listeners = new Set<() => void>()
  private controlSurfaceNote: string | null = null
  private events: StoredRealtimeEvent[] = []
  private nextSequence = 1
  private snapshot: EventStreamSnapshot = {
    controlSurfaceNote: null,
    events: [],
  }

  constructor(private readonly maxEvents: number = DEFAULT_MAX_EVENTS) {}

  getSnapshot(): EventStreamSnapshot {
    return this.snapshot
  }

  subscribe(listener: () => void): () => void {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }

  append(event: Omit<RealtimeEvent, 'id'> | RealtimeEvent): void {
    const sequence = this.nextSequence++
    const normalized: StoredRealtimeEvent = {
      ...event,
      id: 'id' in event ? event.id : `${event.source}:${event.timestamp}:${sequence}`,
      sequence,
    }

    const nextEvents = [...this.events, normalized]
      .sort(compareEvents)
      .slice(-this.maxEvents)

    this.events = nextEvents
    this.refreshSnapshot()
    this.emit()
  }

  noteControlSurfaceGap(note: string): void {
    if (this.controlSurfaceNote === note) {
      return
    }
    this.controlSurfaceNote = note
    this.refreshSnapshot()
    this.emit()
  }

  private emit(): void {
    for (const listener of this.listeners) {
      listener()
    }
  }

  private refreshSnapshot(): void {
    this.snapshot = {
      controlSurfaceNote: this.controlSurfaceNote,
      events: this.events,
    }
  }
}

export interface EventStreamPaneProps {
  readonly controller: EventStreamViewModel
  readonly focused?: boolean
  readonly maxVisibleRows?: number
  readonly sessionId?: string | null
  readonly title?: string
}

export function EventStreamPane(props: EventStreamPaneProps) {
  const state = useSyncExternalStore(
    (listener: () => void) => props.controller.subscribe(listener),
    () => props.controller.getSnapshot(),
  )
  const [filterText, setFilterText] = useState('')
  const [scrollOffset, setScrollOffset] = useState(0)
  const maxVisibleRows = props.maxVisibleRows ?? DEFAULT_VISIBLE_ROWS
  const sessionScopedEvents = useMemo(
    () =>
      props.sessionId
        ? state.events.filter((event) => event.sessionId === props.sessionId)
        : [...state.events],
    [props.sessionId, state.events],
  )
  const filteredEvents = useMemo(
    () => filterEventStreamEvents(sessionScopedEvents, filterText),
    [filterText, sessionScopedEvents],
  )
  const visibleEvents = useMemo(
    () => selectVisibleEvents(filteredEvents, scrollOffset, maxVisibleRows),
    [filteredEvents, maxVisibleRows, scrollOffset],
  )
  const tailFollowing = scrollOffset === 0

  useInput((value, key) => {
    if (props.focused === false) {
      return
    }

    if (key.upArrow) {
      setScrollOffset((current: number) =>
        clampScrollOffset(current + 1, filteredEvents.length, maxVisibleRows),
      )
      return
    }

    if (key.downArrow) {
      setScrollOffset((current: number) =>
        clampScrollOffset(current - 1, filteredEvents.length, maxVisibleRows),
      )
      return
    }

    if (key.pageUp) {
      setScrollOffset((current: number) =>
        clampScrollOffset(current + maxVisibleRows, filteredEvents.length, maxVisibleRows),
      )
      return
    }

    if (key.pageDown) {
      setScrollOffset((current: number) =>
        clampScrollOffset(current - maxVisibleRows, filteredEvents.length, maxVisibleRows),
      )
      return
    }

    if (key.home) {
      setScrollOffset(clampScrollOffset(filteredEvents.length, filteredEvents.length, maxVisibleRows))
      return
    }

    if (key.end) {
      setScrollOffset(0)
      return
    }

    if (key.escape) {
      setFilterText('')
      setScrollOffset(0)
      return
    }

    if (key.backspace || key.delete) {
      setFilterText((current: string) => current.slice(0, -1))
      setScrollOffset(0)
      return
    }

    if (key.ctrl || key.meta || !value || value === '\r' || value === '\n') {
      return
    }

    setFilterText((current: string) => `${current}${value}`)
    setScrollOffset(0)
  })

  return (
    <Box borderColor={REPL_PALETTE.assistant} borderStyle="round" flexDirection="column" paddingX={1}>
      <Box>
        <Text bold color={REPL_PALETTE.assistant}>
          {props.title ?? 'Realtime events'}
        </Text>
        <Spacer />
        <Text color={tailFollowing ? REPL_PALETTE.resolvedAllow : REPL_PALETTE.pending}>
          {tailFollowing ? 'tail on' : 'tail paused'}
        </Text>
      </Box>
      <Text color={REPL_PALETTE.subdued}>
        filter {filterText || '(type acp:, durable:, control:)'}  session {props.sessionId ?? 'all'}
      </Text>
      {state.controlSurfaceNote ? (
        <Text color={REPL_PALETTE.user} wrap="truncate-end">
          control note: {state.controlSurfaceNote}
        </Text>
      ) : null}
      <Box flexDirection="column" marginTop={1}>
        {visibleEvents.length === 0 ? (
          <Text color={REPL_PALETTE.subdued}>No matching events yet.</Text>
        ) : (
          visibleEvents.map((event: RealtimeEvent) => (
            <EventLine event={event} key={event.id} />
          ))
        )}
      </Box>
    </Box>
  )
}

function EventLine(props: { readonly event: RealtimeEvent }) {
  return (
    <Box>
      <Text color={REPL_PALETTE.subdued}>
        [{formatEventTimestamp(props.event.timestamp)}]{' '}
      </Text>
      <Text color={sourceColor(props.event.source)}>
        [{props.event.source}]
      </Text>
      <Text> {props.event.name} </Text>
      <Text color={payloadColor(props.event.severity)} wrap="truncate-end">
        {props.event.payload}
      </Text>
    </Box>
  )
}

export interface EventStreamAcpConnection {
  readonly cancel?: (params: CancelNotification) => Promise<void>
  readonly loadSession: (params: LoadSessionRequest) => Promise<unknown>
  readonly newSession: (params: NewSessionRequest) => Promise<unknown>
  readonly prompt: (params: PromptRequest) => Promise<unknown>
  readonly unstable_resumeSession?: (params: ResumeSessionRequest) => Promise<unknown>
}

export interface AcpEventAdapter {
  readonly connection: EventStreamAcpConnection
  recordCancel(params: CancelNotification): void
  recordNotification(notification: SessionNotification): void
}

export function createAcpEventAdapter(options: {
  readonly connection: EventStreamAcpConnection
  readonly sink: EventStreamSink
}): AcpEventAdapter {
  const record = (
    name: string,
    params: Record<string, unknown>,
    direction: 'in' | 'out',
    sessionId?: string | null,
  ) => {
    options.sink.append({
      timestamp: Date.now(),
      source: 'acp',
      name,
      payload: `${direction === 'in' ? 'dir=in' : 'dir=out'} ${summarizeAcpPayload(params)}`.trim(),
      requestId: extractRequestId(params),
      sessionId,
    })
  }

  return {
    connection: {
      ...options.connection,
      cancel: options.connection.cancel
        ? async (params: CancelNotification) => {
            record('session/cancel', params as Record<string, unknown>, 'out', params.sessionId)
            return options.connection.cancel!(params)
          }
        : undefined,
      loadSession: async (params: LoadSessionRequest) => {
        record('session/load', params as Record<string, unknown>, 'out', params.sessionId)
        return options.connection.loadSession(params)
      },
      newSession: async (params: NewSessionRequest) => {
        record('session/new', params as Record<string, unknown>, 'out', null)
        return options.connection.newSession(params)
      },
      prompt: async (params: PromptRequest) => {
        record('session/prompt', params as Record<string, unknown>, 'out', params.sessionId)
        return options.connection.prompt(params)
      },
      unstable_resumeSession: options.connection.unstable_resumeSession
        ? async (params: ResumeSessionRequest) => {
            record(
              'session/load',
              {
                ...params,
                mode: 'resume',
              },
              'out',
              params.sessionId,
            )
            return options.connection.unstable_resumeSession!(params)
          }
        : undefined,
    },
    recordCancel(params: CancelNotification) {
      record('session/cancel', params as Record<string, unknown>, 'out', params.sessionId)
    },
    recordNotification(notification: SessionNotification) {
      options.sink.append({
        timestamp: Date.now(),
        source: 'acp',
        name: 'session_update',
        payload: `dir=in ${summarizeSessionNotification(notification)}`,
        requestId: notification.update.sessionUpdate === 'usage_update'
          ? null
          : extractRequestId(notification.update as Record<string, unknown>),
        sessionId: notification.sessionId,
      })
    },
  }
}

type DurableEnvelope = {
  readonly headers?: Readonly<Record<string, unknown>>
  readonly key: string
  readonly type: string
  readonly value?: Readonly<Record<string, unknown>>
}

export interface DurableEventSubscription {
  close(): void
}

export interface DurableEventStreamResponse {
  subscribeJson(subscriber: (batch: JsonBatch<DurableEnvelope>) => void): () => void
}

export type DurableEventStreamConnect = (options: {
  readonly headers?: Readonly<Record<string, string>>
  readonly json: true
  readonly live: true
  readonly offset: '-1'
  readonly url: string
}) => Promise<DurableEventStreamResponse>

export async function createDurableEventAdapter(options: {
  readonly connect?: DurableEventStreamConnect
  readonly headers?: Readonly<Record<string, string>>
  readonly sessionId?: string | null
  readonly sink: EventStreamSink
  readonly stateStreamUrl: string
}): Promise<DurableEventSubscription> {
  const connect = options.connect ?? defaultDurableEventStreamConnect
  const response = await connect({
    headers: options.headers,
    json: true,
    live: true,
    offset: '-1',
    url: options.stateStreamUrl,
  })

  const stop = response.subscribeJson((batch: JsonBatch<DurableEnvelope>) => {
    for (const event of mapDurableBatchToEvents(batch, options.sessionId ?? null)) {
      options.sink.append(event)
    }
  })

  return {
    close() {
      stop()
    },
  }
}

export function mapDurableBatchToEvents(
  batch: JsonBatch<DurableEnvelope>,
  sessionId: string | null,
): RealtimeEvent[] {
  const events: RealtimeEvent[] = []

  for (const row of batch.items) {
    const event = mapDurableEnvelopeToEvent(row)
    if (!event) {
      continue
    }
    if (sessionId && event.sessionId !== sessionId) {
      continue
    }
    events.push(event)
  }

  return events
}

function mapDurableEnvelopeToEvent(row: DurableEnvelope): RealtimeEvent | null {
  const collection = mapDurableCollectionName(row.type)
  if (!collection) {
    return null
  }

  const value = row.value ?? {}
  const timestamp = extractTimestamp(value)
  const derivedSessionId = extractSessionId(value)

  return {
    id: `durable:${row.key}:${timestamp}`,
    timestamp,
    source: 'durable',
    name: `${collection}.append`,
    payload: summarizeDurablePayload(collection, row),
    requestId: extractRequestId(value),
    sessionId: derivedSessionId,
  }
}

function mapDurableCollectionName(type: string): string | null {
  switch (type) {
    case 'prompt_request':
      return 'promptTurns'
    case 'permission':
      return 'permissions'
    case 'chunk_v2':
      return 'turnChunks'
    case 'session_v2':
      return 'session_updates'
    default:
      return null
  }
}

export interface ControlEventBus {
  emit(event: {
    readonly name: string
    readonly payload?: string
    readonly sessionId?: string | null
    readonly severity?: EventSeverity
    readonly timestamp?: number
  }): void
  noteMissingSurface(): void
}

export function createControlEventBus(sink: EventStreamSink): ControlEventBus {
  return {
    emit(event) {
      sink.append({
        timestamp: event.timestamp ?? Date.now(),
        source: 'control',
        name: event.name,
        payload: event.payload ?? 'operator lifecycle event',
        sessionId: event.sessionId,
        severity: event.severity ?? 'info',
      })
    },
    noteMissingSurface() {
      sink.noteControlSurfaceGap(
        'Host lifecycle events are not surfaced to TypeScript yet; control rows are CLI-side stubs until that bus lands.',
      )
    },
  }
}

export function filterEventStreamEvents(
  events: readonly RealtimeEvent[],
  filterText: string,
): RealtimeEvent[] {
  const { source, text } = parseFilter(filterText)
  const search = text.toLowerCase()

  return events.filter((event) => {
    if (source && event.source !== source) {
      return false
    }
    if (!search) {
      return true
    }
    return [
      event.name,
      event.payload,
      event.sessionId ?? '',
      event.requestId == null ? '' : String(event.requestId),
    ].some((value) => value.toLowerCase().includes(search))
  })
}

function parseFilter(filterText: string): {
  readonly source: EventSourceTag | null
  readonly text: string
} {
  const trimmed = filterText.trim()
  for (const source of ['acp', 'durable', 'control'] as const) {
    const prefix = `${source}:`
    if (trimmed.toLowerCase().startsWith(prefix)) {
      return {
        source,
        text: trimmed.slice(prefix.length).trim(),
      }
    }
  }

  return {
    source: null,
    text: trimmed,
  }
}

function selectVisibleEvents(
  events: readonly RealtimeEvent[],
  scrollOffset: number,
  maxVisibleRows: number,
): RealtimeEvent[] {
  if (events.length <= maxVisibleRows) {
    return [...events]
  }

  const clampedOffset = clampScrollOffset(scrollOffset, events.length, maxVisibleRows)
  const end = events.length - clampedOffset
  const start = Math.max(0, end - maxVisibleRows)
  return events.slice(start, end)
}

function clampScrollOffset(
  scrollOffset: number,
  totalRows: number,
  visibleRows: number,
): number {
  const maxOffset = Math.max(0, totalRows - visibleRows)
  return Math.max(0, Math.min(scrollOffset, maxOffset))
}

function compareEvents(left: StoredRealtimeEvent, right: StoredRealtimeEvent): number {
  if (left.timestamp !== right.timestamp) {
    return left.timestamp - right.timestamp
  }
  return left.sequence - right.sequence
}

function sourceColor(source: EventSourceTag): string {
  switch (source) {
    case 'acp':
      return REPL_PALETTE.streaming
    case 'control':
      return REPL_PALETTE.user
    case 'durable':
      return REPL_PALETTE.assistant
  }
}

function payloadColor(severity: EventSeverity | undefined): string {
  return severity === 'error' ? REPL_PALETTE.error : REPL_PALETTE.subdued
}

function formatEventTimestamp(timestamp: number): string {
  const date = new Date(timestamp)
  return date.toISOString().slice(11, 23)
}

function summarizeAcpPayload(payload: Record<string, unknown>): string {
  const parts: string[] = []
  if (typeof payload.sessionId === 'string') {
    parts.push(`session=${payload.sessionId}`)
  }
  if ('requestId' in payload) {
    const requestId = extractRequestId(payload)
    if (requestId != null) {
      parts.push(`request=${String(requestId)}`)
    }
  }
  if (typeof payload.cwd === 'string') {
    parts.push(`cwd=${payload.cwd}`)
  }
  if (Array.isArray(payload.prompt)) {
    const promptSummary = payload.prompt
      .map((entry) => summarizePromptPart(entry))
      .filter(Boolean)
      .join(' ')
    if (promptSummary) {
      parts.push(`prompt=${truncate(promptSummary, 48)}`)
    }
  }
  if (typeof payload.mode === 'string') {
    parts.push(`mode=${payload.mode}`)
  }
  return parts.length > 0 ? parts.join(' ') : 'ACP request'
}

function summarizePromptPart(entry: unknown): string {
  if (!entry || typeof entry !== 'object') {
    return ''
  }
  const value = entry as { text?: unknown; type?: unknown }
  if (value.type === 'text' && typeof value.text === 'string') {
    return value.text
  }
  return typeof value.type === 'string' ? value.type : ''
}

function summarizeSessionNotification(notification: SessionNotification): string {
  const update = notification.update as Record<string, unknown>
  const kind = typeof update.sessionUpdate === 'string' ? update.sessionUpdate : 'unknown'
  const parts = [`kind=${kind}`]

  const requestId = extractRequestId(update)
  if (requestId != null) {
    parts.push(`request=${String(requestId)}`)
  }
  if (typeof update.toolCallId === 'string') {
    parts.push(`tool=${update.toolCallId}`)
  }
  if (typeof update.status === 'string') {
    parts.push(`status=${update.status}`)
  }
  if (typeof update.content === 'object' && update.content !== null) {
    const summary = summarizeNotificationContent(update.content as Record<string, unknown>)
    if (summary) {
      parts.push(summary)
    }
  }

  return parts.join(' ')
}

function summarizeNotificationContent(content: Record<string, unknown>): string {
  if (content.type === 'text' && typeof content.text === 'string') {
    return `text=${truncate(content.text, 40)}`
  }
  return typeof content.type === 'string' ? `content=${content.type}` : ''
}

function summarizeDurablePayload(collection: string, row: DurableEnvelope): string {
  const value = row.value ?? {}
  const parts = [`op=${String(row.headers?.operation ?? 'append')}`, `key=${row.key}`]
  const sessionId = extractSessionId(value)
  if (sessionId) {
    parts.push(`session=${sessionId}`)
  }
  const requestId = extractRequestId(value)
  if (requestId != null) {
    parts.push(`request=${String(requestId)}`)
  }
  if (typeof value.kind === 'string') {
    parts.push(`kind=${value.kind}`)
  }
  if (collection === 'turnChunks') {
    const update = value.update
    if (update && typeof update === 'object') {
      const kind = (update as { sessionUpdate?: unknown }).sessionUpdate
      if (typeof kind === 'string') {
        parts.push(`update=${kind}`)
      }
    }
  }
  if (typeof value.state === 'string') {
    parts.push(`state=${value.state}`)
  }
  if (typeof value.title === 'string') {
    parts.push(`title=${truncate(value.title, 24)}`)
  }
  return parts.join(' ')
}

function extractSessionId(value: Readonly<Record<string, unknown>>): string | null {
  return typeof value.sessionId === 'string' ? value.sessionId : null
}

function extractRequestId(value: Readonly<Record<string, unknown>>): string | number | null {
  const requestId = value.requestId
  return typeof requestId === 'string' || typeof requestId === 'number' ? requestId : null
}

function extractTimestamp(value: Readonly<Record<string, unknown>>): number {
  for (const key of ['createdAtMs', 'createdAt', 'updatedAt', 'lastSeenAt'] as const) {
    const candidate = value[key]
    if (typeof candidate === 'number' && Number.isFinite(candidate)) {
      return candidate
    }
  }
  return Date.now()
}

function truncate(value: string, width: number): string {
  return value.length > width ? `${value.slice(0, Math.max(0, width - 1))}…` : value
}

async function defaultDurableEventStreamConnect(options: {
  readonly headers?: Readonly<Record<string, string>>
  readonly json: true
  readonly live: true
  readonly offset: '-1'
  readonly url: string
}): Promise<DurableEventStreamResponse> {
  return stream<DurableEnvelope>({
    headers: options.headers,
    json: options.json,
    live: options.live,
    offset: options.offset,
    url: options.url,
  })
}
