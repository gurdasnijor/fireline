import { agent, compose, middleware, sandbox } from '@fireline/client'
import { approve, trace } from '@fireline/client/middleware'

const agentCommand = (
  process.env.FQA_APPROVAL_AGENT_COMMAND ??
  `${process.cwd()}/target/debug/fireline-testy-fs`
).split(' ')

export default compose(
  sandbox({
    provider: 'local',
    fsBackend: 'streamFs',
    labels: {
      demo: 'fqa-approval',
      surface: 'public-client-api',
    },
  }),
  middleware([
    trace(),
    approve({ scope: 'tool_calls' }),
  ]),
  agent(agentCommand),
)
