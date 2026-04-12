#!/usr/bin/env node
import { spawn } from 'node:child_process'

const { resolveBinary } = await import('../dist/resolve-binary.js')

const bin = resolveBinary({ name: 'fireline-agents', envVar: 'FIRELINE_AGENTS_BIN' })
const child = spawn(bin, process.argv.slice(2), { stdio: 'inherit' })

child.on('exit', (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal)
    return
  }
  process.exit(code ?? 0)
})

child.on('error', (error) => {
  console.error(`fireline-agents: ${(error).message}`)
  process.exit(1)
})
