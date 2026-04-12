import { agent, compose, middleware, sandbox } from '@fireline/client'
import { approve, trace } from '@fireline/client/middleware'

async function main(): Promise<void> {
  // Requires a Fireline host built with `--features anthropic-provider`
  // and ANTHROPIC_API_KEY set in the host environment.
  const handle = await compose(
    sandbox({
      provider: 'anthropic',
      envVars: {
        FIRELINE_ANTHROPIC_NETWORKING_TYPE: 'limited',
        FIRELINE_ANTHROPIC_ALLOWED_HOSTS:
          'https://api.anthropic.com,https://platform.claude.com',
      },
    }),
    middleware([trace(), approve({ scope: 'tool_calls' })]),
    agent(['claude-sonnet-4-6']),
  ).start({ serverUrl: 'http://localhost:4440', name: 'anthropic-provider-demo' })

  console.log('Provisioned Anthropic-backed sandbox:', handle)
  console.log('ACP stream URL:', handle.acp.url)
  console.log('Fireline-relayed state stream URL:', handle.state.url)
}

void main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
