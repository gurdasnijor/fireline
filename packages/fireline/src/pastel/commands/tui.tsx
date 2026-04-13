import React from 'react'
import zod from 'zod'
import { FirelineTuiPreview } from '../tui-preview.js'

export const alias = 'ui'

export const options = zod.object({
  session: zod
    .string()
    .optional()
    .describe('Preview session id override'),
})

type Props = {
  readonly options: zod.infer<typeof options>
}

export default function Tui(props: Props) {
  return <FirelineTuiPreview sessionId={props.options.session ?? 'session-preview'} />
}
