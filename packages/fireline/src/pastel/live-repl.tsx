import { Text, useApp } from 'ink'
import React, { useEffect, useRef, useState } from 'react'
import { FirelineReplApp } from '../repl-ui.js'
import type { ReplViewModel } from '../repl.js'
import { REPL_PALETTE } from '../repl-palette.js'

export interface LiveReplRuntime {
  readonly controller: ReplViewModel
  close(): Promise<void>
}

export interface LiveReplProps {
  readonly start: () => Promise<LiveReplRuntime>
  readonly startingLabel: string
}

export function LiveRepl(props: LiveReplProps) {
  const { exit } = useApp()
  const startRef = useRef(props.start)
  const [runtime, setRuntime] = useState<LiveReplRuntime | null>(null)

  useEffect(() => {
    startRef.current = props.start
  }, [props.start])

  useEffect(() => {
    let disposed = false
    let activeRuntime: LiveReplRuntime | null = null

    void startRef.current()
      .then(async (nextRuntime) => {
        if (disposed) {
          await nextRuntime.close()
          return
        }
        activeRuntime = nextRuntime
        setRuntime(nextRuntime)
      })
      .catch((error: unknown) => {
        if (disposed) {
          return
        }
        const failure = error instanceof Error ? error : new Error(String(error))
        console.error(`fireline: ${failure.message}`)
        process.exitCode = 1
        exit()
      })

    return () => {
      disposed = true
      if (activeRuntime) {
        void activeRuntime.close()
      }
    }
  }, [exit])

  if (!runtime) {
    return <Text color={REPL_PALETTE.subdued}>{props.startingLabel}</Text>
  }

  return (
    <FirelineReplApp
      controller={runtime.controller}
      onExitRequest={(code: number) => {
        process.exitCode = code
        exit()
      }}
      onFailure={(error: Error) => {
        console.error(`fireline: ${error.message}`)
        process.exitCode = 1
        exit()
      }}
    />
  )
}
