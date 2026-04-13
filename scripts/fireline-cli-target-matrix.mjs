import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

const cargoToml = readFileSync(resolve('Cargo.toml'), 'utf8')
const lines = cargoToml.split(/\r?\n/)
const platforms = []
let current = null

for (const rawLine of lines) {
  const line = rawLine.replace(/#.*/, '').trim()
  if (!line) continue

  if (line === '[[workspace.metadata.fireline_cli.platforms]]') {
    if (current) platforms.push(current)
    current = {}
    continue
  }

  if (line.startsWith('[')) {
    if (current) {
      platforms.push(current)
      current = null
    }
    continue
  }

  if (!current) continue

  const match = line.match(/^([A-Za-z0-9_]+)\s*=\s*(.+)$/)
  if (!match) continue

  const [, key, value] = match
  current[key] = parseValue(value)
}

if (current) platforms.push(current)

if (platforms.length === 0) {
  throw new Error('No fireline CLI platform metadata found in Cargo.toml')
}

process.stdout.write(JSON.stringify({ include: platforms }))

function parseValue(raw) {
  const value = raw.trim()
  if (value.startsWith('"') && value.endsWith('"')) {
    return value.slice(1, -1)
  }
  if (value.startsWith('[') && value.endsWith(']')) {
    return value
      .slice(1, -1)
      .split(',')
      .map((entry) => entry.trim())
      .filter(Boolean)
      .map((entry) => parseValue(entry))
  }
  if (value === 'true') return true
  if (value === 'false') return false
  if (/^-?\d+$/.test(value)) return Number.parseInt(value, 10)
  return value
}
