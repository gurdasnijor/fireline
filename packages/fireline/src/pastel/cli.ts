import Pastel from 'pastel'

export async function main(argv: readonly string[] = process.argv): Promise<void> {
  const app = new Pastel({
    description: 'Experimental Fireline Pastel shell for future multi-pane CLI work',
    importMeta: import.meta,
    name: 'fireline-pastel',
  })

  await app.run([...normalizeArgv(argv)])
}

function normalizeArgv(argv: readonly string[]): readonly string[] {
  if (argv.length >= 2) {
    return argv
  }

  return ['node', 'fireline-pastel', ...argv]
}
