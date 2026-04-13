import { Box, Spacer, Text, useApp, useInput } from 'ink'
import React, { useEffect, useState, useSyncExternalStore } from 'react'
import { logReplDebug } from './repl-debug.js'
import { REPL_PALETTE } from './repl-palette.js'
import {
  ConversationPane,
  type ConversationApprovalAction,
} from './repl-pane-conversation.js'
import { EventStreamPane, type EventStreamViewModel } from './repl-pane-events.js'
import { SessionStatePane } from './repl-pane-state.js'
import type {
  ReplViewModel,
  ReplViewState,
  TranscriptEntry,
} from './repl.js'

export function FirelineReplApp(props: {
  readonly controller: ReplViewModel
  readonly events: EventStreamViewModel
  readonly onExitRequest: (code: number) => void
  readonly onFailure: (error: Error) => void
}) {
  const state = useSyncExternalStore(
    (listener: () => void) => props.controller.subscribe(listener),
    () => props.controller.getSnapshot(),
  )
  const { exit } = useApp()
  const [approvalFocus, setApprovalFocus] = useState<ConversationApprovalAction>('allow')
  const [input, setInput] = useState('')
  const spinner = useSpinner(state.busy || state.pendingTools > 0 || state.adminBusy)
  const { committedEntries, liveEntries } = partitionTranscriptEntries(state)
  const selectedDetached =
    Boolean(state.selectedSessionId) && state.selectedSessionId !== state.sessionId

  useEffect(() => {
    if (state.pendingApproval) {
      setApprovalFocus('allow')
    }
  }, [state.pendingApproval?.requestId])

  const resolveFocusedApproval = (action: ConversationApprovalAction) => {
    void props.controller
      .resolvePendingApproval(action === 'allow')
      .catch((error: unknown) => {
        props.onFailure(error instanceof Error ? error : new Error(String(error)))
        exit()
      })
  }

  useInput((value, key) => {
    logReplDebug('ui.key', {
      approvalFocus,
      busy: state.busy,
      inputLength: input.length,
      key,
      pendingApproval: Boolean(state.pendingApproval),
      resolvingApproval: state.resolvingApproval,
      value,
    })

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
      if (state.resolvingApproval) {
        logReplDebug('ui.approval.ignored.resolving', { value })
        return
      }

      if (key.tab || key.leftArrow || key.rightArrow) {
        setApprovalFocus((current: ConversationApprovalAction) => nextApprovalFocus(current))
        return
      }

      if (key.return || value === ' ') {
        resolveFocusedApproval(approvalFocus)
        return
      }

      if (key.ctrl || key.meta || !value) {
        return
      }

      if (value.toLowerCase() === 'a') {
        setApprovalFocus('allow')
        resolveFocusedApproval('allow')
        return
      }

      if (value.toLowerCase() === 'd') {
        setApprovalFocus('deny')
        resolveFocusedApproval('deny')
        return
      }

      return
    }

    if (state.busy || state.adminBusy) {
      return
    }

    if (input.length === 0 && value === '[') {
      props.controller.selectPreviousSession()
      return
    }

    if (input.length === 0 && value === ']') {
      props.controller.selectNextSession()
      return
    }

    if (input.length === 0 && value === 'l') {
      void props.controller.attachSelectedSession().catch((error: unknown) => {
        props.onFailure(error instanceof Error ? error : new Error(String(error)))
      })
      return
    }

    if (input.length === 0 && value === 'x') {
      void props.controller.stopRuntime().catch((error: unknown) => {
        props.onFailure(error instanceof Error ? error : new Error(String(error)))
      })
      return
    }

    if (input.length === 0 && value === 'R') {
      void props.controller.restartRuntime().catch((error: unknown) => {
        props.onFailure(error instanceof Error ? error : new Error(String(error)))
      })
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
    <Box flexDirection="column" paddingX={1}>
      <TopBar state={state} spinner={spinner} />
      <Box flexDirection="row" marginTop={1}>
        <Box flexDirection="column" flexGrow={3} marginRight={1}>
          {selectedDetached ? <DetachedSessionNotice sessionId={state.selectedSessionId} /> : null}
          <ConversationPane
            committedEntries={committedEntries}
            focusedApprovalAction={approvalFocus}
            input={input}
            liveEntries={liveEntries}
            spinner={spinner}
            state={state}
            title="Conversation"
          />
        </Box>
        <Box flexDirection="column" flexGrow={2} minWidth={42}>
          <AdminCard state={state} />
          <SessionStatePane
            acpUrl={state.acpUrl}
            db={state.db}
            runtimeId={state.runtimeId}
            serverUrl={state.serverUrl}
            sessionId={state.selectedSessionId}
            stateStreamUrl={state.stateStreamUrl}
          />
          <EventStreamPane
            controller={props.events}
            focused={false}
            sessionId={state.selectedSessionId}
            title="Realtime events"
          />
        </Box>
      </Box>
    </Box>
  )
}

function TopBar(props: {
  readonly state: ReplViewState
  readonly spinner: string
}) {
  const statusColor = props.state.pendingApproval
    ? REPL_PALETTE.pending
    : props.state.busy || props.state.adminBusy
      ? REPL_PALETTE.streaming
      : props.state.pendingTools > 0
        ? REPL_PALETTE.pending
        : REPL_PALETTE.resolvedAllow

  return (
    <Box borderColor={REPL_PALETTE.assistant} borderStyle="round" flexDirection="column" paddingX={1}>
      <Box>
        <Text bold color={REPL_PALETTE.assistant}>
          Fireline TUI
        </Text>
        <Spacer />
        <Text color={statusColor}>
          {props.state.busy || props.state.pendingTools > 0 || props.state.pendingApproval || props.state.adminBusy
            ? `${props.spinner} live`
            : 'ready'}
        </Text>
      </Box>
      <Box>
        <Text color={REPL_PALETTE.subdued}>
          runtime {shortIdentifier(props.state.runtimeId)}
        </Text>
        <Spacer />
        <Text color={REPL_PALETTE.subdued}>
          {hostLabel(props.state.serverUrl)}  {runtimeStatusLabel(props.state)}
        </Text>
      </Box>
      <Box>
        {props.state.sessionTabs.length > 0 ? (
          <SessionTabs state={props.state} />
        ) : (
          <Text color={REPL_PALETTE.subdued}>No session tabs yet.</Text>
        )}
      </Box>
      <Box>
        <Text color={REPL_PALETTE.subdued}>active tools {props.state.pendingTools}</Text>
        <Spacer />
        <Text color={REPL_PALETTE.subdued}>{renderUsage(props.state)}</Text>
      </Box>
    </Box>
  )
}

function SessionTabs(props: { readonly state: ReplViewState }) {
  return (
    <Box>
      {props.state.sessionTabs.map((tab, index) => (
        <Box
          borderColor={
            tab.sessionId === props.state.selectedSessionId
              ? REPL_PALETTE.assistant
              : REPL_PALETTE.subdued
          }
          borderStyle="round"
          key={tab.sessionId}
          marginRight={index === props.state.sessionTabs.length - 1 ? 0 : 1}
          paddingX={1}
        >
          <Text
            bold={tab.sessionId === props.state.selectedSessionId}
            color={tab.pendingApprovals > 0 ? REPL_PALETTE.pending : REPL_PALETTE.subdued}
          >
            {shortIdentifier(tab.sessionId)}
            {tab.attached ? ' *' : ''}
            {tab.pendingApprovals > 0 ? ` a${tab.pendingApprovals}` : ''}
            {tab.activePrompts > 0 ? ` p${tab.activePrompts}` : ''}
            {tab.state ? ` ${tab.state}` : ''}
          </Text>
        </Box>
      ))}
    </Box>
  )
}

function DetachedSessionNotice(props: { readonly sessionId: string | null }) {
  return (
    <Box
      borderColor={REPL_PALETTE.pending}
      borderStyle="round"
      flexDirection="column"
      marginBottom={1}
      paddingX={1}
    >
      <Text color={REPL_PALETTE.pending}>
        Session {props.sessionId ?? 'unknown'} is selected in the tabs but not attached in ACP.
      </Text>
      <Text color={REPL_PALETTE.subdued}>
        Press l to load or resume it into the conversation pane.
      </Text>
    </Box>
  )
}

function AdminCard(props: { readonly state: ReplViewState }) {
  const stopColor =
    props.state.runtimeStatus === 'stopped' ? REPL_PALETTE.resolvedDeny : REPL_PALETTE.user
  return (
    <Box
      borderColor={REPL_PALETTE.streaming}
      borderStyle="round"
      flexDirection="column"
      marginBottom={1}
      paddingX={1}
    >
      <Text bold color={REPL_PALETTE.streaming}>
        Admin controls
      </Text>
      <Text color={REPL_PALETTE.subdued}>
        [ / ] session tabs  l load/resume selected  x stop runtime  R restart runtime
      </Text>
      <Text color={REPL_PALETTE.subdued}>
        attached {props.state.sessionId ?? 'none'}  selected {props.state.selectedSessionId ?? 'none'}
      </Text>
      <Box marginTop={1}>
        <ActionBadge color={REPL_PALETTE.assistant} label="l load/resume" />
        <Box marginLeft={1}>
          <ActionBadge color={stopColor} label="x stop" />
        </Box>
        <Box marginLeft={1}>
          <ActionBadge
            color={
              props.state.supportsRuntimeRestart ? REPL_PALETTE.streaming : REPL_PALETTE.subdued
            }
            label={props.state.supportsRuntimeRestart ? 'R restart' : 'R restart unavailable'}
          />
        </Box>
      </Box>
      {props.state.adminMessage ? (
        <Text color={props.state.adminBusy ? REPL_PALETTE.pending : REPL_PALETTE.subdued}>
          {props.state.adminMessage}
        </Text>
      ) : null}
    </Box>
  )
}

function ActionBadge(props: { readonly color: string; readonly label: string }) {
  return (
    <Box borderColor={props.color} borderStyle="round" paddingX={1}>
      <Text color={props.color}>{props.label}</Text>
    </Box>
  )
}

function hostLabel(serverUrl: string): string {
  try {
    return new URL(serverUrl).host
  } catch {
    return serverUrl
  }
}

function runtimeStatusLabel(state: ReplViewState): string {
  if (state.adminBusy) {
    return 'admin busy'
  }
  if (state.runtimeStatus) {
    return `runtime ${state.runtimeStatus}`
  }
  return 'runtime unknown'
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

function nextApprovalFocus(
  current: ConversationApprovalAction,
): ConversationApprovalAction {
  return current === 'allow' ? 'deny' : 'allow'
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
