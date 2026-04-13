import React from 'react'
import { Box, Text } from 'ink'
import { REPL_PALETTE } from '../../repl-palette.js'

export const description =
  'Run specs locally, build hosted images, deploy them, or connect to a running Fireline host.'

export default function Index() {
  return (
    <Box flexDirection="column">
      <Text color={REPL_PALETTE.subdued}>Shorthand: fireline {'<file.ts>'} is the same as fireline run {'<file.ts>'}.</Text>
      <Box marginTop={1} flexDirection="column">
        <Text bold color={REPL_PALETTE.assistant}>
          Available commands
        </Text>
        <Text>  run {'<file.ts>'}     Boot conductor + streams and provision a spec locally</Text>
        <Text>  build {'<file.ts>'}   Build a hosted Fireline OCI image</Text>
        <Text>  deploy {'<file.ts>'}  Build and hand off to a target-native deploy CLI</Text>
        <Text>  repl [session-id]  Connect to a running Fireline ACP host</Text>
        <Text>  agents add {'<id>'}   Install an ACP agent from the public registry</Text>
      </Box>
      <Box marginTop={1} flexDirection="column">
        <Text color={REPL_PALETTE.subdued}>
          Use --help on any command to inspect flags. The live REPL now runs
          under the same Pastel entrypoint instead of a parallel preview binary.
        </Text>
      </Box>
    </Box>
  )
}
