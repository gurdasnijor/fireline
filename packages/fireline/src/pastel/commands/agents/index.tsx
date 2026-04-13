import React from 'react'
import { Box, Text } from 'ink'
import { REPL_PALETTE } from '../../../repl-palette.js'

export const description = 'Install ACP agents from the public registry.'

export default function AgentsIndex() {
  return (
    <Box flexDirection="column">
      <Text>Use fireline agents add {'<id>'} to install a registry agent.</Text>
      <Box marginTop={1} flexDirection="column">
        <Text bold color={REPL_PALETTE.assistant}>
          Available commands
        </Text>
        <Text>  add {'<id>'}   Install an ACP agent by registry id</Text>
      </Box>
      <Box marginTop={1}>
        <Text color={REPL_PALETTE.subdued}>
          FIRELINE_AGENTS_BIN can override the bundled fireline-agents binary.
        </Text>
      </Box>
    </Box>
  )
}
