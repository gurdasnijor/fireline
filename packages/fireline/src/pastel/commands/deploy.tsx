import React from 'react'
import { argument, option } from 'pastel'
import zod from 'zod'
import {
  deploy,
  parseArgs,
} from '../../cli.js'
import { CommandRunner } from '../command-runner.js'
import { pushStringFlag } from '../argv.js'

export const description =
  'Build a hosted image and hand it off to a target-native deploy CLI.'

export const args = zod
  .tuple([
    zod.string().describe(argument({
      description: 'Spec file to deploy.',
      name: 'file.ts',
    })),
  ])
  .rest(
    zod.string().describe(argument({
      description: 'Arguments passed through to the native deploy CLI after --.',
      name: 'native-arg',
    })),
  )

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
    description: 'Hosted target from ~/.fireline/hosted.json.',
    valueDescription: 'name',
  })),
  to: zod.string().optional().describe(option({
    description: 'Native deploy target: fly | cloudflare-containers | docker-compose | k8s.',
    valueDescription: 'platform',
  })),
  token: zod.string().optional().describe(option({
    description: 'Override deploy auth token for the resolved target.',
    valueDescription: 'value',
  })),
})

type Props = {
  readonly args: [string, ...string[]]
  readonly options: DeployOptions
}

type DeployOptions = zod.infer<typeof options>

export default function DeployCommand(props: Props) {
  const [file, ...nativeArgs] = props.args

  return (
    <CommandRunner
      label={`Deploying ${file}...`}
      task={async () => {
        const parsed = parseArgs(createDeployArgv(file, nativeArgs, props.options))
        return await deploy(parsed)
      }}
    />
  )
}

function createDeployArgv(
  file: string,
  nativeArgs: readonly string[],
  options: DeployOptions,
): readonly string[] {
  const argv = ['deploy', file]
  pushStringFlag(argv, '--to', options.to)
  pushStringFlag(argv, '--target', options.target)
  pushStringFlag(argv, '--token', options.token)
  pushStringFlag(argv, '--state-stream', options.stateStream)
  pushStringFlag(argv, '--name', options.name)
  pushStringFlag(argv, '--provider', options.provider)
  if (nativeArgs.length > 0) {
    argv.push('--', ...nativeArgs)
  }
  return argv
}
