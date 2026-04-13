import assert from 'node:assert/strict'
import test from 'node:test'
import { renderToString } from 'ink'
import React from 'react'
import { FirelineTuiPreview } from './tui-preview.js'

test('pastel TUI preview composes the three pane scaffold', () => {
  const output = renderToString(
    React.createElement(FirelineTuiPreview, {
      sessionId: 'session-preview',
    }),
    { columns: 160 },
  )

  assert.match(output, /Pane 1 · Conversation/)
  assert.match(output, /Pane 2 · Materialized state/)
  assert.match(output, /Pane 3 · Realtime events/)
  assert.match(output, /Tool Approval/)
  assert.match(output, /Session state/)
  assert.match(output, /control note:/)
})
