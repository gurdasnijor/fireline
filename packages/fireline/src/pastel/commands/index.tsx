import React from 'react'
import { Box, Text } from 'ink'
import { REPL_PALETTE } from '../../repl-palette.js'

export default function Index() {
  return (
    <Box flexDirection="column">
      <Text>
        This is a parallel Pastel scaffold for Fireline. The stable CLI remains
        the existing <Text color={REPL_PALETTE.assistant}>fireline</Text> binary.
      </Text>
      <Box marginTop={1} flexDirection="column">
        <Text bold color={REPL_PALETTE.assistant}>
          Available commands
        </Text>
        <Text>  tui      render a seeded three-pane TUI preview</Text>
      </Box>
      <Box marginTop={1} flexDirection="column">
        <Text color={REPL_PALETTE.subdued}>
          Use this scaffold to iterate on command layout, shared app wrappers,
          and multi-pane TUI composition without rewriting the current CLI
          surface first.
        </Text>
      </Box>
    </Box>
  )
}
