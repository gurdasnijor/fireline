#!/usr/bin/env node
import { existsSync } from 'node:fs'
import { dirname, resolve as resolvePath } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const packageRoot = dirname(dirname(fileURLToPath(import.meta.url)))
const sourceEntry = resolvePath(packageRoot, 'src/cli.ts')
const distEntry = resolvePath(packageRoot, 'dist/cli.js')

if (existsSync(sourceEntry)) {
  const { tsImport } = await import('tsx/esm/api')
  const moduleUrl = pathToFileURL(sourceEntry).href
  const parentURL = pathToFileURL(`${packageRoot}/`).href
  const mod = await tsImport(moduleUrl, { parentURL })
  await mod.main(process.argv.slice(2))
} else {
  const { main } = await import(pathToFileURL(distEntry).href)
  await main(process.argv.slice(2))
}
