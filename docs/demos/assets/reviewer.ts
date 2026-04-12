import { compose, agent, sandbox, middleware } from '@fireline/client'
import { approve, peer, secretsProxy, trace } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'

export default compose(
  sandbox({ resources: [localPath('.', '/workspace', true)] }),
  middleware([
    trace(),
    approve({ scope: 'tool_calls' }),
    secretsProxy({
      ANTHROPIC_API_KEY: { ref: 'env:ANTHROPIC_API_KEY' },
    }),
    peer({ peers: ['agent'] }),
  ]),
  agent(['pi-acp']),
)
