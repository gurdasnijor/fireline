import assert from 'node:assert/strict'
import test from 'node:test'
import { parseArgs } from './cli.js'

test('parseArgs parses deploy flags', () => {
  const args = parseArgs([
    'deploy',
    'agent.ts',
    '--remote',
    'https://agents.example.com',
    '--token',
    'token-123',
    '--name',
    'reviewer',
    '--state-stream',
    'session-1',
  ])

  assert.equal(args.command, 'deploy')
  assert.equal(args.helpFor, 'deploy')
  assert.equal(args.file, 'agent.ts')
  assert.equal(args.remote, 'https://agents.example.com')
  assert.equal(args.token, 'token-123')
  assert.equal(args.name, 'reviewer')
  assert.equal(args.stateStream, 'session-1')
})

test('parseArgs rejects deploy without remote', () => {
  assert.throws(
    () => parseArgs(['deploy', 'agent.ts']),
    /deploy requires --remote <url>/,
  )
})

test('parseArgs returns deploy-specific help topic', () => {
  const args = parseArgs(['deploy', '--help'])
  assert.equal(args.command, 'help')
  assert.equal(args.helpFor, 'deploy')
})
