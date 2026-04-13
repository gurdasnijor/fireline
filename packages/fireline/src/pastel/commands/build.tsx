import React from 'react'
import { argument, option } from 'pastel'
import zod from 'zod'
import {
  build,
  parseArgs,
} from '../../cli.js'
import { CommandRunner } from '../command-runner.js'
import {
  pushStringFlag,
} from '../argv.js'

export const description = 'Build a hosted Fireline OCI image from a spec.'

export const args = zod.tuple([
  zod.string().describe(argument({
    description: 'Spec file to build.',
    name: 'file.ts',
  })),
])

export const options = zod.object({
  name: zod.string().optional().describe(option({
    description: 'Override deployment name baked into the spec.',
    valueDescription: 'name',
  })),
  provider: zod.string().optional().describe(option({
    description: 'Override sandbox.provider baked into the spec.',
    valueDescription: 'provider',
  })),
  stateStream: zod.string().optional().describe(option({
    description: 'Override durable state stream name baked into the spec.',
    valueDescription: 'stream',
  })),
  target: zod.string().optional().describe(option({
    description: 'Scaffold target config: cloudflare | docker | docker-compose | fly | k8s.',
    valueDescription: 'platform',
  })),
})

type Props = {
  readonly args: [string]
  readonly options: BuildOptions
}

type BuildOptions = zod.infer<typeof options>

export default function BuildCommand(props: Props) {
  return (
    <CommandRunner
      label={`Building ${props.args[0]}...`}
      task={async () => {
        const parsed = parseArgs(createBuildArgv(props.args[0], props.options))
        return await build(parsed)
      }}
    />
  )
}

function createBuildArgv(
  file: string,
  options: BuildOptions,
): readonly string[] {
  const argv = ['build', file]
  pushStringFlag(argv, '--target', options.target)
  pushStringFlag(argv, '--state-stream', options.stateStream)
  pushStringFlag(argv, '--name', options.name)
  pushStringFlag(argv, '--provider', options.provider)
  return argv
}
