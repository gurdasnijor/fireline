import React from 'react'
import { argument, option } from 'pastel'
import zod from 'zod'
import {
  parseArgs,
  run,
  startHostedRunSession,
} from '../../cli.js'
import { attachRepl } from '../../repl.js'
import { CommandRunner } from '../command-runner.js'
import {
  pushBooleanFlag,
  pushNumberFlag,
  pushStringFlag,
} from '../argv.js'
import { LiveRepl } from '../live-repl.js'

export const description = 'Boot Fireline locally and provision a spec.'

export const args = zod.tuple([
  zod.string().describe(argument({
    description: 'Spec file to run.',
    name: 'file.ts',
  })),
])

export const options = zod.object({
  name: zod.string().optional().describe(option({
    description: 'Logical agent name.',
    valueDescription: 'name',
  })),
  port: zod.number().optional().describe(option({
    description: 'ACP control-plane port.',
    valueDescription: 'n',
  })),
  provider: zod.string().optional().describe(option({
    description: 'Override sandbox.provider from the spec.',
    valueDescription: 'provider',
  })),
  repl: zod.boolean().default(false).describe(option({
    description: 'Start an interactive REPL after booting the host.',
  })),
  stateStream: zod.string().optional().describe(option({
    description: 'Explicit durable state stream name.',
    valueDescription: 'stream',
  })),
  streamsPort: zod.number().optional().describe(option({
    description: 'Durable-streams port.',
    valueDescription: 'n',
  })),
})

type Props = {
  readonly args: [string]
  readonly options: RunOptions
}

type RunOptions = zod.infer<typeof options>

export default function RunCommand(props: Props) {
  if (!props.options.repl) {
    return (
      <CommandRunner
        label={`Running ${props.args[0]}...`}
        task={async () => {
          const parsed = parseArgs(createRunArgv(props.args[0], props.options))
          return await run(parsed)
        }}
      />
    )
  }

  return (
    <LiveRepl
      start={async () => {
        const parsed = parseArgs(createRunArgv(props.args[0], props.options))
        const started = await startHostedRunSession(parsed)
        try {
          const attachment = await attachRepl({
            acpUrl: started.handle.acp.url,
            runtimeId: started.handle.id,
            serverUrl: `http://127.0.0.1:${parsed.port}`,
            sessionId: started.sessionId ?? null,
            stateStreamUrl: started.handle.state.url,
          })
          return {
            controller: attachment.controller,
            close: async () => {
              await attachment.close()
              await started.close()
            },
          }
        } catch (error) {
          await started.close()
          throw error
        }
      }}
      startingLabel={`Booting ${props.args[0]} and attaching the REPL...`}
    />
  )
}

function createRunArgv(
  file: string,
  options: RunOptions,
): readonly string[] {
  const argv = ['run', file]
  pushNumberFlag(argv, '--port', options.port)
  pushBooleanFlag(argv, '--repl', options.repl)
  pushNumberFlag(argv, '--streams-port', options.streamsPort)
  pushStringFlag(argv, '--state-stream', options.stateStream)
  pushStringFlag(argv, '--name', options.name)
  pushStringFlag(argv, '--provider', options.provider)
  return argv
}
