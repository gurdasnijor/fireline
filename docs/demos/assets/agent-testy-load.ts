// Demo fallback spec — used ONLY IF pi-acp provisioning hasn't cleared
// by rehearsal (see mono-dls P0 bug + operator script §Fallbacks).
//
// Same middleware chain as docs/demos/assets/agent.ts — the only
// difference is the agent command. Narrative holds: "same 15-line
// spec"; only the real-model beat degrades to a deterministic echo.
//
// Operator decision gate: pre-flight P11 verifies agent(['pi-acp']) on
// a scratch spec before the show. If it fails, swap to this file.

import { compose, agent, sandbox, middleware } from '@fireline/client'
import { approve, budget, peer, secretsProxy, trace } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'

export default compose(
  sandbox({ resources: [localPath('.', '/workspace')] }),
  middleware([
    trace(),
    approve({ scope: 'tool_calls' }),
    budget({ tokens: 2_000_000 }),
    secretsProxy({
      ANTHROPIC_API_KEY: { ref: 'env:ANTHROPIC_API_KEY' },
      GITHUB_TOKEN: { ref: 'secret:gh-pat', allow: 'api.github.com' },
    }),
    peer({ peers: ['reviewer'] }),
  ]),
  agent([process.env.AGENT_COMMAND ?? '../../target/debug/fireline-testy-load']),
)
