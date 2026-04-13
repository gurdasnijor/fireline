import { Text, useApp } from 'ink'
import React, { useEffect, useRef } from 'react'
import { REPL_PALETTE } from '../repl-palette.js'

export interface CommandRunnerProps {
  readonly label: string
  readonly task: () => Promise<number | void>
}

export function CommandRunner(props: CommandRunnerProps) {
  const { exit } = useApp()
  const taskRef = useRef(props.task)

  useEffect(() => {
    taskRef.current = props.task
  }, [props.task])

  useEffect(() => {
    let active = true

    void taskRef.current()
      .then((code) => {
        if (!active) {
          return
        }
        process.exitCode = code ?? 0
        exit()
      })
      .catch((error: unknown) => {
        if (!active) {
          return
        }
        const failure = error instanceof Error ? error : new Error(String(error))
        console.error(`fireline: ${failure.message}`)
        process.exitCode = 1
        exit()
      })

    return () => {
      active = false
    }
  }, [exit])

  return <Text color={REPL_PALETTE.subdued}>{props.label}</Text>
}
