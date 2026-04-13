import React from 'react'
import type { AppProps } from 'pastel'
import { Box, Spacer, Text } from 'ink'
import { REPL_PALETTE } from '../../repl-palette.js'

export default function App({ Component, commandProps }: AppProps) {
  return (
    <Box flexDirection="column" paddingX={1}>
      <Box
        borderColor={REPL_PALETTE.assistant}
        borderStyle="round"
        flexDirection="column"
        paddingX={1}
      >
        <Box>
          <Text bold color={REPL_PALETTE.assistant}>
            Fireline Pastel
          </Text>
          <Spacer />
          <Text color={REPL_PALETTE.pending}>experimental</Text>
        </Box>
        <Text color={REPL_PALETTE.subdued}>
          File-based command scaffold for future multi-pane CLI/TUI work.
        </Text>
      </Box>
      <Box marginTop={1}>
        <Component {...commandProps} />
      </Box>
    </Box>
  )
}
