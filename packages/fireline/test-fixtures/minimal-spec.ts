import { agent, compose, middleware, sandbox } from '@fireline/client'
import { trace } from '@fireline/client/middleware'

export default compose(
  sandbox({ labels: { test: 'cli-smoke' } }),
  middleware([trace()]),
  agent(['/bin/sh', '-c', 'while true; do sleep 1; done']),
)
