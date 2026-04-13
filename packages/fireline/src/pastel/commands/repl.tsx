import React from 'react'
import { argument } from 'pastel'
import zod from 'zod'
import {
  decorateStandaloneReplError,
  looksLikeSpecPath,
  parseArgs,
} from '../../cli.js'
import { attachRepl } from '../../repl.js'
import { LiveRepl } from '../live-repl.js'

export const description = 'Connect to a running Fireline ACP host.'

export const args = zod.tuple([
  zod.string().optional().describe(argument({
    description: 'Existing session id to resume or load.',
    name: 'session-id',
  })),
]).optional()

type Props = {
  readonly args?: [string?]
  readonly options: Record<string, never>
}

export default function ReplCommand(props: Props) {
  return (
    <LiveRepl
      start={async () => {
        const sessionId = props.args?.[0] ?? null
        const parsed = parseArgs(sessionId ? ['repl', sessionId] : ['repl'])

        if (parsed.sessionId && looksLikeSpecPath(parsed.sessionId)) {
          throw new Error(
            `${parsed.sessionId} looks like a spec path, not a session id. Did you mean: fireline run ${parsed.sessionId} --repl ?`,
          )
        }

        try {
          return await attachRepl({ sessionId: parsed.sessionId })
        } catch (error) {
          throw decorateStandaloneReplError(error)
        }
      }}
      startingLabel={
        props.args?.[0]
          ? `Attaching to session ${props.args[0]}...`
          : 'Connecting to the running Fireline host...'
      }
    />
  )
}
