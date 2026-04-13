import { useApp, useInput } from 'ink'
import React, {
  useEffect,
  useState,
  useSyncExternalStore,
} from 'react'
import { logReplDebug } from './repl-debug.js'
import {
  ConversationPane,
  type ConversationApprovalAction,
} from './repl-pane-conversation.js'
import type {
  ReplViewModel,
  ReplViewState,
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
  const [approvalFocus, setApprovalFocus] = useState<ConversationApprovalAction>('allow')
  const [input, setInput] = useState('')
  const spinner = useSpinner(state.busy || state.pendingTools > 0)
  const { committedEntries, liveEntries } = partitionTranscriptEntries(state)

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

      if (key.tab) {
        setApprovalFocus((current: ConversationApprovalAction) => nextApprovalFocus(current))
        logReplDebug('ui.approval.focus.tab', {
          next: nextApprovalFocus(approvalFocus),
          shift: Boolean(key.shift),
        })
        return
      }

      if (key.leftArrow || key.rightArrow) {
        setApprovalFocus((current: ConversationApprovalAction) => nextApprovalFocus(current))
        logReplDebug('ui.approval.focus.arrow', {
          next: nextApprovalFocus(approvalFocus),
        })
        return
      }

      if (key.return || value === ' ') {
        logReplDebug('ui.approval.resolve.focused', { action: approvalFocus })
        resolveFocusedApproval(approvalFocus)
        return
      }

      if (key.ctrl || key.meta || !value) {
        logReplDebug('ui.approval.ignored.nontext', { key })
        return
      }

      if (value.toLowerCase() === 'a') {
        setApprovalFocus('allow')
        logReplDebug('ui.approval.resolve.hotkey', { action: 'allow' })
        resolveFocusedApproval('allow')
        return
      }

      if (value.toLowerCase() === 'd') {
        setApprovalFocus('deny')
        logReplDebug('ui.approval.resolve.hotkey', { action: 'deny' })
        resolveFocusedApproval('deny')
        return
      }

      logReplDebug('ui.approval.ignored.composer_key', { value })
      return
    }

    if (state.busy) {
      logReplDebug('ui.input.ignored.busy', { value })
      return
    }

    if (key.return) {
      const currentInput = input
      setInput('')
      void props.controller
        .submit(currentInput)
        .then((result: 'ignored' | 'quit' | 'sent') => {
          logReplDebug('ui.input.submit.result', { result })
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
    <ConversationPane
      committedEntries={committedEntries}
      focusedApprovalAction={approvalFocus}
      input={input}
      liveEntries={liveEntries}
      spinner={spinner}
      state={state}
    />
  )
}
function nextApprovalFocus(current: ConversationApprovalAction): ConversationApprovalAction {
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
