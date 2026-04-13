import assert from 'node:assert/strict'
import test from 'node:test'
import { normalizePastelArgv } from './argv.js'

test('normalizePastelArgv rewrites spec shorthand into run', () => {
  assert.deepEqual(
    normalizePastelArgv(['node', 'fireline', 'docs/demos/assets/agent.ts', '--repl']),
    ['node', 'fireline', 'run', 'docs/demos/assets/agent.ts', '--repl'],
  )
})

test('normalizePastelArgv preserves explicit top-level commands', () => {
  assert.deepEqual(
    normalizePastelArgv(['node', 'fireline', 'deploy', 'agent.ts', '--to', 'fly']),
    ['node', 'fireline', 'deploy', 'agent.ts', '--to', 'fly'],
  )
})

test('normalizePastelArgv preserves root help and version flags', () => {
  assert.deepEqual(normalizePastelArgv(['node', 'fireline', '--help']), ['node', 'fireline', '--help'])
  assert.deepEqual(normalizePastelArgv(['node', 'fireline', '--version']), ['node', 'fireline', '--version'])
})
