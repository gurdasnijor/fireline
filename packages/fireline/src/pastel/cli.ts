import Pastel from 'pastel'
import { normalizePastelArgv } from './argv.js'

export async function main(argv: readonly string[] = process.argv): Promise<void> {
  const app = new Pastel({
    description: 'Run specs locally, build hosted images, deploy them, or connect to a running host',
    importMeta: import.meta,
    name: 'fireline',
  })

  await app.run([...normalizePastelArgv(argv)])
}
