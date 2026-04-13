import fireline, { agent, compose, middleware, sandbox } from '@fireline/client'
import { trace } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'
import { extractChunkTextPreview } from '@fireline/state'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const question = 'Can an agent review a PR or file diff without silently changing the repo?'
const serverUrl = process.env.FIRELINE_URL ?? 'http://127.0.0.1:4440'
const repoPath =
  process.env.REPO_PATH ??
  resolve(fileURLToPath(new URL('.', import.meta.url)), '../..')
const agentCommand = (
  process.env.AGENT_COMMAND ?? 'npx -y @agentclientprotocol/claude-agent-acp'
).split(' ')
const reviewPrompt =
  process.env.REVIEW_PROMPT ??
  'Review the current changes in /workspace as if this were a pull request. Give me the top three merge risks, the files I should inspect first, and one follow-up question for the author.'

const handle = await compose(
  sandbox({
    provider: 'local',
    resources: [localPath(repoPath, '/workspace', true)],
    envVars: process.env.ANTHROPIC_API_KEY
      ? { ANTHROPIC_API_KEY: process.env.ANTHROPIC_API_KEY }
      : undefined,
    labels: { demo: 'code-review-agent' },
  }),
  middleware([trace()]),
  agent(agentCommand),
).start({ serverUrl, name: 'code-review-agent' })

const db = await fireline.db({ stateStreamUrl: handle.state.url })
const acp = await handle.connect('code-review-agent')

try {
  const { sessionId } = await acp.newSession({ cwd: '/workspace', mcpServers: [] })
  await acp.prompt({
    sessionId,
    prompt: [{ type: 'text', text: reviewPrompt }],
  })

  const reviewText = await waitForReviewText(sessionId, 5_000)

  console.log(
    JSON.stringify(
      {
        question,
        repoPath,
        sessionId,
        stateStream: handle.state.url,
        readOnlyWorkspace: '/workspace',
        reviewText,
      },
      null,
      2,
    ),
  )
} finally {
  await acp.close()
  db.close()
  await handle.destroy()
}

async function waitForReviewText(sessionId: string, timeoutMs: number): Promise<string> {
  const immediate = currentReviewText(sessionId)
  if (immediate) return immediate

  return await new Promise((resolve) => {
    const timeout = setTimeout(() => {
      subscription.unsubscribe()
      resolve(currentReviewText(sessionId))
    }, timeoutMs)

    const subscription = db.chunks.subscribe(() => {
      const text = currentReviewText(sessionId)
      if (!text) return
      clearTimeout(timeout)
      subscription.unsubscribe()
      resolve(text)
    })
  })
}

function currentReviewText(sessionId: string): string {
  return db.chunks.toArray
    .filter((row) => row.sessionId === sessionId)
    .map((row) => extractChunkTextPreview(row.update))
    .filter((text) => text.length > 0)
    .join('')
}
