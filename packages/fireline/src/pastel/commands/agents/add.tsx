import React from 'react'
import { argument } from 'pastel'
import zod from 'zod'
import {
  parseArgs,
  runAgents,
} from '../../../cli.js'
import { CommandRunner } from '../../command-runner.js'

export const description = 'Install an ACP agent by registry id.'

export const args = zod.tuple([
  zod.string().describe(argument({
    description: 'Registry id to install.',
    name: 'id',
  })),
])

type Props = {
  readonly args: [string]
  readonly options: Record<string, never>
}

export default function AgentsAddCommand(props: Props) {
  return (
    <CommandRunner
      label={`Installing agent ${props.args[0]}...`}
      task={async () => {
        const parsed = parseArgs(['agents', 'add', props.args[0]])
        return await runAgents(parsed)
      }}
    />
  )
}
