import type { ToolCallStatus } from '@agentclientprotocol/sdk'
import { Box, Spacer, Static, Text } from 'ink'
import React from 'react'
import { REPL_PALETTE } from './repl-palette.js'
import type {
  MessageEntry,
  PendingApproval,
  PlanEntry,
  ReplViewState,
  ToolEntry,
  TranscriptEntry,
} from './repl.js'

export type ConversationApprovalAction = 'allow' | 'deny'

export interface ConversationPaneProps {
  readonly committedEntries: readonly TranscriptEntry[]
  readonly focusedApprovalAction: ConversationApprovalAction
  readonly input: string
  readonly liveEntries: readonly TranscriptEntry[]
  readonly spinner: string
  readonly state: ReplViewState
  readonly title?: string
}

export function ConversationPane(props: ConversationPaneProps) {
  return (
    <>
      {/* Keep committed turns outside Ink's hot render path so terminal
          scrollback remains the durable reading artifact. */}
      <Static items={[...props.committedEntries]}>
        {(entry: TranscriptEntry) => <EntryView entry={entry} key={entry.id} />}
      </Static>
      <Box flexDirection="column" paddingX={1}>
        <Header
          state={props.state}
          spinner={props.spinner}
          title={props.title ?? 'Conversation'}
        />
        {props.state.pendingApproval ? (
          <ApprovalCard
            focusedAction={props.focusedApprovalAction}
            pending={props.state.pendingApproval}
            resolving={props.state.resolvingApproval}
            state={props.state}
          />
        ) : null}
        <Box flexDirection="column" marginTop={1}>
          {props.committedEntries.length === 0 && props.liveEntries.length === 0 ? (
            <EmptyState />
          ) : props.liveEntries.length > 0 ? (
            props.liveEntries.map((entry: TranscriptEntry) => (
              <EntryView entry={entry} key={entry.id} />
            ))
          ) : (
            <ScrollbackNotice />
          )}
        </Box>
        <Composer
          busy={props.state.busy}
          input={props.input}
          pendingApproval={props.state.pendingApproval}
          resolvingApproval={props.state.resolvingApproval}
          spinner={props.spinner}
        />
        <StatusBar state={props.state} />
      </Box>
    </>
  )
}

function Header(props: {
  readonly state: ReplViewState
  readonly spinner: string
  readonly title: string
}) {
  const statusColor = props.state.pendingApproval
    ? REPL_PALETTE.pending
    : props.state.busy
      ? REPL_PALETTE.streaming
      : props.state.pendingTools > 0
        ? REPL_PALETTE.pending
        : REPL_PALETTE.resolvedAllow

  return (
    <Box
      borderColor={REPL_PALETTE.assistant}
      borderStyle="round"
      flexDirection="column"
      paddingX={1}
    >
      <Box>
        <Text bold color={REPL_PALETTE.assistant}>
          {props.title}
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
    <Box
      borderColor={REPL_PALETTE.subdued}
      borderStyle="round"
      flexDirection="column"
      paddingX={1}
    >
      <Text color={REPL_PALETTE.subdued}>Connected.</Text>
      <Text color={REPL_PALETTE.subdued}>
        Type a prompt below and press Enter to send it.
      </Text>
    </Box>
  )
}

function ScrollbackNotice() {
  return (
    <Box
      borderColor={REPL_PALETTE.subdued}
      borderStyle="round"
      flexDirection="column"
      paddingX={1}
    >
      <Text color={REPL_PALETTE.subdued}>
        Earlier conversation lives in terminal scrollback.
      </Text>
      <Text color={REPL_PALETTE.subdued}>
        Type a new prompt below to continue the session.
      </Text>
    </Box>
  )
}

function ApprovalCard(props: {
  readonly focusedAction: ConversationApprovalAction
  readonly pending: PendingApproval
  readonly resolving: boolean
  readonly state: ReplViewState
}) {
  const card = buildApprovalCard(props.pending, props.state.entries)
  const borderColor = props.resolving ? REPL_PALETTE.streaming : REPL_PALETTE.pending

  return (
    <Box
      borderColor={borderColor}
      borderStyle="round"
      flexDirection="column"
      marginTop={1}
      paddingX={1}
    >
      <Text bold color={borderColor}>
        Tool Approval
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
      <Box marginTop={1}>
        <ApprovalActionButton
          focused={props.focusedAction === 'allow'}
          label="Accept"
          tone={REPL_PALETTE.resolvedAllow}
        />
        <Box marginLeft={1}>
          <ApprovalActionButton
            focused={props.focusedAction === 'deny'}
            label="Decline"
            tone={REPL_PALETTE.resolvedDeny}
          />
        </Box>
      </Box>
      <Text color={REPL_PALETTE.subdued}>
        {props.resolving
          ? 'Resolving approval...'
          : 'Tab to focus actions. Enter or Space confirms. a = accept, d = decline.'}
      </Text>
    </Box>
  )
}

function ApprovalActionButton(props: {
  readonly focused: boolean
  readonly label: string
  readonly tone: string
}) {
  return (
    <Box
      borderColor={props.focused ? props.tone : REPL_PALETTE.subdued}
      borderStyle="round"
      paddingX={1}
    >
      <Text bold={props.focused} color={props.focused ? props.tone : REPL_PALETTE.subdued}>
        {props.label}
      </Text>
    </Box>
  )
}

function EntryView(props: { readonly entry: TranscriptEntry }) {
  switch (props.entry.kind) {
    case 'message':
      return <MessageView entry={props.entry} />
    case 'plan':
      return <PlanView entry={props.entry} />
    case 'tool':
      return <ToolView entry={props.entry} />
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
    <Box
      borderColor={color}
      borderStyle="round"
      flexDirection="column"
      marginBottom={1}
      paddingX={1}
    >
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
    <Box
      borderColor={color}
      borderStyle="round"
      flexDirection="column"
      marginBottom={1}
      paddingX={1}
    >
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
    <Box
      borderColor={REPL_PALETTE.streaming}
      borderStyle="round"
      flexDirection="column"
      marginBottom={1}
      paddingX={1}
    >
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
            ? 'Approval is resolving. Composer is locked until the decision completes.'
            : 'Approval pending. Composer is locked; resolve the approval card first.'
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
