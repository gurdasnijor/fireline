import { compose, agent, sandbox, middleware } from '@fireline/client'
import { approve, budget, peer, secretsProxy, trace } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'

export default compose(
  sandbox({ resources: [localPath('.', '/workspace')] }),
  middleware([
    trace(),
    approve({ scope: 'tool_calls' }),
    budget({ tokens: 2_000_000 }),
    peer({ peers: ['reviewer'] }),
  ]),
  agent(['npx', '-y', '@agentclientprotocol/claude-agent-acp']),
)
