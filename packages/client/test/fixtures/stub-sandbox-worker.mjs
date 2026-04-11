import { createInterface } from 'node:readline'

const rl = createInterface({ input: process.stdin, crlfDelay: Infinity })

rl.on('line', (line) => {
  if (!line.trim()) return
  const request = JSON.parse(line)
  process.stdout.write(`${JSON.stringify({ kind: 'ok', value: { echoed: request.arguments } })}\n`)
})

rl.on('close', () => process.exit(0))
process.on('SIGTERM', () => process.exit(0))
