import type { ToolCallStatus } from '@agentclientprotocol/sdk'
import {
  Box,
  Spacer,
  Text,
  useApp,
  useInput,
  useWindowSize,
} from 'ink'
import React, {
  useEffect,
  useState,
  useSyncExternalStore,
} from 'react'
import type {
  MessageEntry,
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
    (listener) => props.controller.subscribe(listener),
    () => props.controller.getSnapshot(),
  )
  const { exit } = useApp()
  const { rows } = useWindowSize()
  const [input, setInput] = useState('')
  const spinner = useSpinner(state.busy || state.pendingTools > 0)
  const visibleEntries = state.entries.slice(-visibleEntryCount(rows))

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

    if (state.busy) {
      return
    }

    if (key.return) {
      const currentInput = input
      setInput('')
      void props.controller
        .submit(currentInput)
        .then((result) => {
          if (result === 'quit') {
            props.onExitRequest(0)
            exit()
          }
        })
        .catch((error) => {
          props.onFailure(error instanceof Error ? error : new Error(String(error)))
          exit()
        })
      return
    }

    if (key.backspace || key.delete) {
      setInput((current) => current.slice(0, -1))
      return
    }

    if (key.escape) {
      setInput('')
      return
    }

    if (key.ctrl || key.meta || !value) {
      return
    }

    setInput((current) => `${current}${value}`)
  })

  return (
    <Box flexDirection="column" paddingX={1}>
      <Header state={state} spinner={spinner} />
      <Box flexDirection="column" marginTop={1}>
        {visibleEntries.length === 0 ? (
          <EmptyState />
        ) : (
          visibleEntries.map((entry) => <EntryView entry={entry} key={entry.id} />)
        )}
      </Box>
      <Composer busy={state.busy} input={input} spinner={spinner} />
    </Box>
  )
}

function Header(props: {
  readonly state: ReplViewState
  readonly spinner: string
}) {
  return (
    <Box borderColor="cyan" borderStyle="round" flexDirection="column" paddingX={1}>
      <Box>
        <Text bold color="cyan">
          Fireline REPL
        </Text>
        <Spacer />
        <Text color={props.state.busy ? 'yellow' : props.state.pendingTools > 0 ? 'magenta' : 'green'}>
          {props.state.busy || props.state.pendingTools > 0
            ? `${props.spinner} live`
            : 'ready'}
        </Text>
      </Box>
      <Box>
        <Text color="gray">
          session {props.state.sessionId ?? 'connecting'}
        </Text>
        <Spacer />
        <Text color="gray">
          {hostLabel(props.state.serverUrl)}
        </Text>
      </Box>
      <Box>
        <Text color="gray">
          active tools {props.state.pendingTools}
        </Text>
        <Spacer />
        <Text color="gray">{renderUsage(props.state)}</Text>
      </Box>
    </Box>
  )
}

function EmptyState() {
  return (
    <Box borderColor="gray" borderStyle="round" flexDirection="column" paddingX={1}>
      <Text color="gray">Connected.</Text>
      <Text color="gray">Type a prompt below and press Enter to send it.</Text>
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
      ? 'cyan'
      : props.entry.role === 'thought'
        ? 'yellow'
        : 'magenta'
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
        <Text color="gray">{props.entry.toolKind ?? 'operation'}</Text>
      </Box>
      <Text>{props.entry.title}</Text>
      {props.entry.detail ? (
        <Text color="gray">{props.entry.detail}</Text>
      ) : null}
    </Box>
  )
}

function PlanView(props: { readonly entry: PlanEntry }) {
  return (
    <Box borderColor="blue" borderStyle="round" flexDirection="column" marginBottom={1} paddingX={1}>
      <Text bold color="blue">
        plan
      </Text>
      {props.entry.items.map((item, index) => (
        <Text color="gray" key={`${props.entry.id}:${index}`}>
          - {item}
        </Text>
      ))}
    </Box>
  )
}

function Composer(props: {
  readonly busy: boolean
  readonly input: string
  readonly spinner: string
}) {
  return (
    <Box
      borderColor={props.busy ? 'yellow' : 'gray'}
      borderStyle="round"
      flexDirection="column"
      marginTop={1}
      paddingX={1}
    >
      <Text color="gray">
        {props.busy
          ? `${props.spinner} waiting for the running session...`
          : 'Enter to send, Esc to clear, Ctrl+C or /quit to exit.'}
      </Text>
      <Text color={props.busy ? 'gray' : 'white'}>
        <Text color="cyan">&gt;</Text>{' '}
        {props.input.length > 0 ? props.input : 'Ask the running host something...'}
      </Text>
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

function toolStatusColor(status: ToolCallStatus): string {
  switch (status) {
    case 'completed':
      return 'green'
    case 'failed':
      return 'red'
    case 'in_progress':
      return 'yellow'
    case 'pending':
    default:
      return 'magenta'
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

function visibleEntryCount(rows: number): number {
  return Math.max(4, rows - 10)
}
