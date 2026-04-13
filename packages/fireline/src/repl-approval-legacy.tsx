import { Box, Text } from 'ink'
import React from 'react'
import { REPL_PALETTE } from './repl-palette.js'
import type { PendingApproval } from './repl.js'

/**
 * Legacy mono-thnc.13 approval prompt kept as reference while the card kit
 * replaces the active REPL approval surface.
 */
export function LegacyApprovalPrompt(props: {
  readonly pending: PendingApproval
  readonly resolving: boolean
}) {
  return (
    <Box
      borderColor={REPL_PALETTE.pending}
      borderStyle="round"
      flexDirection="column"
      marginTop={1}
      paddingX={1}
    >
      <Text bold color={REPL_PALETTE.pending}>
        approval pending
      </Text>
      <Text>{props.pending.summary}</Text>
      <Text color={REPL_PALETTE.subdued}>
        {props.resolving ? 'resolving approval...' : 'Press y to allow or n to deny.'}
      </Text>
    </Box>
  )
}
