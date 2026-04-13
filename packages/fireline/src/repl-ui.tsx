import type { ToolCallStatus } from '@agentclientprotocol/sdk'
import {
  Box,
  Static,
  Spacer,
  Text,
  useApp,
  useInput,
} from 'ink'
import React, {
  useEffect,
  useState,
  useSyncExternalStore,
} from 'react'
import { REPL_PALETTE } from './repl-palette.js'
import type {
  MessageEntry,
  PendingApproval,
  PlanEntry,
  ReplViewModel,
  ReplViewState,
  ToolEntry,
  TranscriptEntry,
} from './repl.js'

export function FirelineReplApp(props: {
  readonly controller: ReplViewModel
  readonly onExitRequest: (code: number) => void
  readonly onFailure: (error: Error) => void
}) {
  const state = useSyncExternalStore(
    (listener: () => void) => props.controller.subscribe(listener),
    () => props.controller.getSnapshot(),
  )
  const { exit } = useApp()
  const [input, setInput] = useState('')
  const spinner = useSpinner(state.busy || state.pendingTools > 0)
  const { committedEntries, liveEntries } = partitionTranscriptEntries(state)

  useInput((value, key) => {
    if (key.ctrl && value === 'c') {
      props.onExitRequest(130)
      exit()
      return
    }

    if (key.ctrl && value === 'd' && input.length === 0) {
      props.onExitRequest(0)
      exit()
      return
    }

    if (state.pendingApproval) {
      if (key.ctrl || key.meta || !value) {
        return
      }

      if (value.toLowerCase() === 'y') {
        void props.controller.resolvePendingApproval(true).catch((error: unknown) => {
          props.onFailure(error instanceof Error ? error : new Error(String(error)))
          exit()
        })
      } else if (value.toLowerCase() === 'n') {
        void props.controller.resolvePendingApproval(false).catch((error: unknown) => {
          props.onFailure(error instanceof Error ? error : new Error(String(error)))
          exit()
        })
      }
      return
    }

    if (state.busy) {
      return
    }

    if (key.return) {
      const currentInput = input
      setInput('')
      void props.controller
        .submit(currentInput)
        .then((result: 'ignored' | 'quit' | 'sent') => {
          if (result === 'quit') {
            props.onExitRequest(0)
            exit()
          }
        })
        .catch((error: unknown) => {
          props.onFailure(error instanceof Error ? error : new Error(String(error)))
          exit()
        })
      return
    }

    if (key.backspace || key.delete) {
      setInput((current: string) => current.slice(0, -1))
      return
    }

    if (key.escape) {
      setInput('')
      return
    }

    if (key.ctrl || key.meta || !value) {
      return
    }

    setInput((current: string) => `${current}${value}`)
  })

  return (
    <>
      <Static items={[...committedEntries]}>
        {(entry: TranscriptEntry) => <EntryView entry={entry} key={entry.id} />}
      </Static>
      <Box flexDirection="column" paddingX={1}>
        <Header state={state} spinner={spinner} />
        {state.pendingApproval ? (
          <ApprovalPrompt
            pending={state.pendingApproval}
            resolving={state.resolvingApproval}
            state={state}
          />
        ) : null}
        <Box flexDirection="column" marginTop={1}>
          {committedEntries.length === 0 && liveEntries.length === 0 ? (
            <EmptyState />
          ) : (
            liveEntries.map((entry: TranscriptEntry) => (
              <EntryView entry={entry} key={entry.id} />
            ))
          )}
        </Box>
        <Composer
          busy={state.busy}
          input={input}
          pendingApproval={state.pendingApproval}
          resolvingApproval={state.resolvingApproval}
          spinner={spinner}
        />
        <StatusBar state={state} />
      </Box>
    </>
  )
}

function Header(props: {
  readonly state: ReplViewState
  readonly spinner: string
}) {
  const statusColor = props.state.pendingApproval
    ? REPL_PALETTE.pending
    : props.state.busy
      ? REPL_PALETTE.streaming
      : props.state.pendingTools > 0
        ? REPL_PALETTE.pending
        : REPL_PALETTE.resolvedAllow

  return (
    <Box borderColor={REPL_PALETTE.assistant} borderStyle="round" flexDirection="column" paddingX={1}>
      <Box>
        <Text bold color={REPL_PALETTE.assistant}>
          Fireline REPL
        </Text>
        <Spacer />
        <Text color={statusColor}>
          {props.state.busy || props.state.pendingTools > 0 || props.state.pendingApproval
            ? `${props.spinner} live`
            : 'ready'}
        </Text>
      </Box>
      <Box>
        <Text color={REPL_PALETTE.subdued}>
          session {props.state.sessionId ?? 'connecting'}
        </Text>
        <Spacer />
        <Text color={REPL_PALETTE.subdued}>
          {hostLabel(props.state.serverUrl)}
        </Text>
      </Box>
      <Box>
        <Text color={REPL_PALETTE.subdued}>
          active tools {props.state.pendingTools}
        </Text>
        <Spacer />
        <Text color={REPL_PALETTE.subdued}>{renderUsage(props.state)}</Text>
      </Box>
    </Box>
  )
}

function EmptyState() {
  return (
    <Box borderColor={REPL_PALETTE.subdued} borderStyle="round" flexDirection="column" paddingX={1}>
      <Text color={REPL_PALETTE.subdued}>Connected.</Text>
      <Text color={REPL_PALETTE.subdued}>Type a prompt below and press Enter to send it.</Text>
    </Box>
  )
}

function ApprovalPrompt(props: {
  readonly pending: PendingApproval
  readonly resolving: boolean
  readonly state: ReplViewState
}) {
  const card = buildApprovalCard(props.pending, props.state.entries)

  return (
    <Box
      borderColor={REPL_PALETTE.pending}
      borderStyle="round"
      flexDirection="column"
      marginTop={1}
      paddingX={1}
    >
      <Text bold color={REPL_PALETTE.pending}>
        approval pending
      </Text>
      <Text bold>{card.toolName}</Text>
      <Text color={REPL_PALETTE.subdued}>
        request {String(props.pending.requestId)}
        {props.pending.toolCallId ? `  tool ${props.pending.toolCallId}` : ''}
      </Text>
      <Box flexDirection="column" marginTop={1}>
        <Text color={REPL_PALETTE.subdued}>arguments</Text>
        {card.argumentLines.map((line: string, index: number) => (
          <Text key={`approval-arg:${index}`}>  {line}</Text>
        ))}
      </Box>
      <Text color={REPL_PALETTE.subdued} italic>
        reason: {card.reason}
      </Text>
      <Text>
        {props.resolving ? (
          <Text color={REPL_PALETTE.streaming}>resolving approval...</Text>
        ) : (
          <>
            <Text color={REPL_PALETTE.pending}>[y]</Text>
            <Text color={REPL_PALETTE.resolvedAllow}> approve</Text>
            <Text color={REPL_PALETTE.subdued}> / </Text>
            <Text color={REPL_PALETTE.pending}>[n]</Text>
            <Text color={REPL_PALETTE.resolvedDeny}> deny</Text>
          </>
        )}
      </Text>
    </Box>
  )
}

function EntryView(props: { readonly entry: TranscriptEntry }) {
  switch (props.entry.kind) {
    case 'message':
      return <MessageView entry={props.entry} />
    case 'tool':
      return <ToolView entry={props.entry} />
    case 'plan':
      return <PlanView entry={props.entry} />
  }
}

function MessageView(props: { readonly entry: MessageEntry }) {
  const color =
    props.entry.role === 'assistant'
      ? REPL_PALETTE.assistant
      : props.entry.role === 'thought'
        ? REPL_PALETTE.streaming
        : REPL_PALETTE.user
  const title =
    props.entry.role === 'assistant'
      ? 'assistant'
      : props.entry.role === 'thought'
        ? 'thinking'
        : 'you'

  return (
    <Box borderColor={color} borderStyle="round" flexDirection="column" marginBottom={1} paddingX={1}>
      <Text bold color={color}>
        {title}
      </Text>
      <Text>{props.entry.text}</Text>
    </Box>
  )
}

function ToolView(props: { readonly entry: ToolEntry }) {
  const color = toolStatusColor(props.entry.status)

  return (
    <Box borderColor={color} borderStyle="round" flexDirection="column" marginBottom={1} paddingX={1}>
      <Box>
        <Text bold color={color}>
          tool {props.entry.status}
        </Text>
        <Spacer />
        <Text color={REPL_PALETTE.subdued}>{props.entry.toolKind ?? 'operation'}</Text>
      </Box>
      <Text>{props.entry.title}</Text>
      {props.entry.detail ? (
        <Text color={REPL_PALETTE.subdued}>{props.entry.detail}</Text>
      ) : null}
    </Box>
  )
}

function PlanView(props: { readonly entry: PlanEntry }) {
  return (
    <Box borderColor={REPL_PALETTE.streaming} borderStyle="round" flexDirection="column" marginBottom={1} paddingX={1}>
      <Text bold color={REPL_PALETTE.streaming}>
        plan
      </Text>
      {props.entry.items.map((item: string, index: number) => (
        <Text color={REPL_PALETTE.subdued} key={`${props.entry.id}:${index}`}>
          - {item}
        </Text>
      ))}
    </Box>
  )
}

function Composer(props: {
  readonly busy: boolean
  readonly input: string
  readonly pendingApproval: ReplViewState['pendingApproval']
  readonly resolvingApproval: boolean
  readonly spinner: string
}) {
  const borderColor = props.pendingApproval
    ? REPL_PALETTE.pending
    : props.busy
      ? REPL_PALETTE.streaming
      : REPL_PALETTE.subdued

  return (
    <Box
      borderColor={borderColor}
      borderStyle="round"
      flexDirection="column"
      marginTop={1}
      paddingX={1}
    >
      <Text color={REPL_PALETTE.subdued}>
        {props.pendingApproval
          ? props.resolvingApproval
            ? 'Resolving approval and waiting for the running session...'
            : 'Approval pending. Press y to allow or n to deny.'
          : props.busy
            ? `${props.spinner} waiting for the running session...`
            : 'Enter to send, Esc to clear, Ctrl+C or /quit to exit.'}
      </Text>
      <Text color={props.busy || props.pendingApproval ? REPL_PALETTE.subdued : 'white'}>
        <Text color={REPL_PALETTE.assistant}>&gt;</Text>{' '}
        {props.input.length > 0 ? props.input : 'Ask the running host something...'}
      </Text>
    </Box>
  )
}

function StatusBar(props: { readonly state: ReplViewState }) {
  return (
    <Box marginTop={1}>
      <Text color={REPL_PALETTE.subdued} dimColor>
        {`session:${shortIdentifier(props.state.sessionId)}  runtime:${shortRuntimeId(props.state.runtimeId)}  acp:${acpPort(props.state.acpUrl)}  stream:${shortStreamLabel(props.state.stateStreamUrl)}`}
      </Text>
    </Box>
  )
}

function buildApprovalCard(
  pending: PendingApproval,
  entries: readonly TranscriptEntry[],
): {
  readonly argumentLines: readonly string[]
  readonly reason: string
  readonly toolName: string
} {
  const latestTool = findLatestTool(entries, pending.toolCallId)
  const latestUser = findLatestUserMessage(entries)
  const parsedPrompt = parsePromptArguments(latestUser?.text)
  const argumentSource =
    parsedPrompt.argumentsValue ??
    (latestTool?.detail ? { detail: latestTool.detail } : { prompt: pending.summary })

  return {
    argumentLines: prettyJsonLines(argumentSource),
    reason: pending.reason ?? 'awaiting operator decision',
    toolName:
      latestTool?.title ??
      parsedPrompt.toolName ??
      pending.toolCallId ??
      'prompt approval',
  }
}

function findLatestTool(
  entries: readonly TranscriptEntry[],
  toolCallId: string | null,
): ToolEntry | null {
  for (let index = entries.length - 1; index >= 0; index -= 1) {
    const entry = entries[index]
    if (entry.kind !== 'tool') {
      continue
    }
    if (!toolCallId || entry.toolCallId === toolCallId) {
      return entry
    }
  }

  return null
}

function findLatestUserMessage(entries: readonly TranscriptEntry[]): MessageEntry | null {
  for (let index = entries.length - 1; index >= 0; index -= 1) {
    const entry = entries[index]
    if (entry.kind === 'message' && entry.role === 'user') {
      return entry
    }
  }

  return null
}

function parsePromptArguments(text: string | undefined): {
  readonly argumentsValue: unknown
  readonly toolName: string | null
} {
  if (!text) {
    return {
      argumentsValue: { prompt: '' },
      toolName: null,
    }
  }

  const trimmed = text.trim()
  try {
    const parsed = JSON.parse(trimmed)
    if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
      const record = parsed as Record<string, unknown>
      const toolName = firstString(record.command, record.tool, record.name)
      if (toolName) {
        const { command: _command, name: _name, tool: _tool, ...rest } = record
        return {
          argumentsValue: Object.keys(rest).length > 0 ? rest : record,
          toolName,
        }
      }

      return {
        argumentsValue: record,
        toolName: null,
      }
    }
  } catch {}

  return {
    argumentsValue: { prompt: trimmed },
    toolName: null,
  }
}

function firstString(...values: Array<unknown>): string | null {
  for (const value of values) {
    if (typeof value === 'string' && value.length > 0) {
      return value
    }
  }

  return null
}

function prettyJsonLines(value: unknown): readonly string[] {
  return JSON.stringify(value, null, 2).split('\n')
}

function hostLabel(serverUrl: string): string {
  try {
    return new URL(serverUrl).host
  } catch {
    return serverUrl
  }
}

function renderUsage(state: ReplViewState): string {
  if (!state.usage || state.usage.size <= 0) {
    return 'usage n/a'
  }

  const width = 12
  const ratio = Math.max(0, Math.min(1, state.usage.used / state.usage.size))
  const filled = Math.round(ratio * width)
  const bar = `${'#'.repeat(filled)}${'.'.repeat(Math.max(0, width - filled))}`
  return `ctx [${bar}] ${state.usage.used}/${state.usage.size}`
}

function shortIdentifier(value: string | null): string {
  if (!value) {
    return 'unknown'
  }

  const candidate = value.includes(':') ? value.split(':').at(-1) ?? value : value
  if (candidate.length <= 12) {
    return candidate
  }

  return candidate.slice(0, 8)
}

function shortRuntimeId(value: string | null): string {
  return shortIdentifier(value)
}

function acpPort(acpUrl: string): string {
  try {
    const url = new URL(acpUrl)
    return url.port || (url.protocol === 'wss:' ? '443' : '80')
  } catch {
    return '?'
  }
}

function shortStreamLabel(stateStreamUrl: string | null): string {
  if (!stateStreamUrl) {
    return 'unknown'
  }

  try {
    const url = new URL(stateStreamUrl)
    const tail = url.pathname.split('/').filter(Boolean).at(-1) ?? url.pathname
    return `${url.host}/${truncateMiddle(tail, 28)}`
  } catch {
    return truncateMiddle(stateStreamUrl, 36)
  }
}

function truncateMiddle(value: string, maxLength: number): string {
  if (value.length <= maxLength) {
    return value
  }

  const side = Math.max(4, Math.floor((maxLength - 1) / 2))
  return `${value.slice(0, side)}…${value.slice(-side)}`
}

function toolStatusColor(status: ToolCallStatus): string {
  switch (status) {
    case 'completed':
      return REPL_PALETTE.resolvedAllow
    case 'failed':
      return REPL_PALETTE.error
    case 'in_progress':
      return REPL_PALETTE.streaming
    case 'pending':
    default:
      return REPL_PALETTE.pending
  }
}

function useSpinner(active: boolean): string {
  const frames = ['-', '\\', '|', '/']
  const [index, setIndex] = useState(0)

  useEffect(() => {
    if (!active) {
      setIndex(0)
      return
    }

    const timer = setInterval(() => {
      setIndex((current) => (current + 1) % frames.length)
    }, 80)

    return () => {
      clearInterval(timer)
    }
  }, [active])

  return active ? frames[index] : 'o'
}

export function partitionTranscriptEntries(state: ReplViewState): {
  readonly committedEntries: readonly TranscriptEntry[]
  readonly liveEntries: readonly TranscriptEntry[]
} {
  if (!hasActiveTurn(state)) {
    return {
      committedEntries: state.entries,
      liveEntries: [],
    }
  }

  const liveStartIndex = findLiveTurnStart(state.entries)
  return {
    committedEntries: state.entries.slice(0, liveStartIndex),
    liveEntries: state.entries.slice(liveStartIndex),
  }
}

function hasActiveTurn(state: ReplViewState): boolean {
  return (
    state.busy ||
    state.pendingTools > 0 ||
    state.pendingApproval !== null ||
    state.resolvingApproval
  )
}

function findLiveTurnStart(entries: readonly TranscriptEntry[]): number {
  for (let index = entries.length - 1; index >= 0; index -= 1) {
    const entry = entries[index]
    if (entry.kind === 'message' && entry.role === 'user') {
      return index
    }
  }

  return entries.length
}
