import { compose, agent, sandbox, middleware } from '@fireline/client'
import { budget, peer, trace } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'

export default compose(
  sandbox({ resources: [localPath('.', '/workspace')] }),
  middleware([
    trace(),
    budget({ tokens: 2_000_000 }),
    peer({ peers: ['reviewer'] }),
  ]),
  agent(['npx', '-y', '@agentclientprotocol/claude-agent-acp']),
)
