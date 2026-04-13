import assert from 'node:assert/strict'
import test from 'node:test'
import { renderToString } from 'ink'
import React from 'react'
import { FirelineReplApp } from './repl-ui.js'
import { ReplController } from './repl.js'

test('repl controller submits prompts and renders streamed output', async () => {
  const prompts: string[] = []

  const controller = new ReplController({
    sendPrompt: async (text) => {
      prompts.push(text)
      controller.receiveNotification({
        sessionId: 'session-123',
        update: {
          sessionUpdate: 'agent_message_chunk',
          content: {
            type: 'text',
            text: 'Hello back from Fireline.',
          },
        },
      })
    },
    serverUrl: 'http://127.0.0.1:4440',
    sessionId: 'session-123',
  })

  const result = await controller.submit('Ping the host')
  const output = renderToString(
    React.createElement(FirelineReplApp, {
      controller,
      onExitRequest: () => {},
      onFailure: () => {},
    }),
    { columns: 80 },
  )

  assert.equal(result, 'sent')
  assert.deepEqual(prompts, ['Ping the host'])
  assert.match(output, /Ping the host/)
  assert.match(output, /assistant/i)
  assert.match(output, /Hello back from Fireline\./)
})
