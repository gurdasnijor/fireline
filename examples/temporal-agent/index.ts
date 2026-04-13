import {
  AwakeableAlreadyResolvedError,
  resolveAwakeable,
  sessionCompletionKey,
  workflowContext,
} from '@fireline/client'

type ChangeWindowResolution = {
  readonly note: string
  readonly openedBy: string
  readonly window: string
}

const question =
  'Can my agent pause for an overnight change window and resume cleanly tomorrow?'

const command = process.argv[2] ?? 'wait'

switch (command) {
  case 'wait':
    await waitForChangeWindow()
    break
  case 'resolve':
    await resolveChangeWindow()
    break
  default:
    printUsage()
    process.exitCode = 1
}

async function waitForChangeWindow(): Promise<void> {
  const stateStreamUrl = requiredEnv('STATE_STREAM_URL')
  const sessionId = requiredEnv('SESSION_ID')
  const key = sessionCompletionKey(sessionId)
  const ctx = workflowContext({ stateStreamUrl })

  emit({
    question,
    resumeHint:
      'You can stop this process and run the same wait command later. The durable wait is keyed by sessionId, not by this PID.',
    sessionId,
    stateStreamUrl,
    status: 'waiting',
    story:
      'The planning run is done. Fireline can stay parked in the durable stream until the overnight change window opens.',
    windowKey: key,
  })

  const resolution = await ctx.awakeable<ChangeWindowResolution>(key).promise

  emit({
    question,
    resolution,
    sessionId,
    stateStreamUrl,
    status: 'resumed',
    story:
      'The same logical wait resumed from the durable stream after the change window opened.',
  })
}

async function resolveChangeWindow(): Promise<void> {
  const stateStreamUrl = requiredEnv('STATE_STREAM_URL')
  const sessionId = requiredEnv('SESSION_ID')
  const resolution: ChangeWindowResolution = {
    note:
      process.env.RESOLUTION_NOTE ??
      'Ops opened the nightly change window. Continue the rollout.',
    openedBy: process.env.OPENED_BY ?? 'ops-oncall',
    window: process.env.CHANGE_WINDOW ?? 'tonight-02:00',
  }

  try {
    await resolveAwakeable({
      headers: undefined,
      key: sessionCompletionKey(sessionId),
      streamUrl: stateStreamUrl,
      traceContext: traceContextFromEnv(),
      value: resolution,
    })
  } catch (error) {
    if (error instanceof AwakeableAlreadyResolvedError) {
      emit({
        alreadyResolved: true,
        question,
        sessionId,
        stateStreamUrl,
        status: 'already-resolved',
      })
      return
    }
    throw error
  }

  emit({
    question,
    resolution,
    sessionId,
    stateStreamUrl,
    status: 'resolved',
    story:
      'An external process appended the durable completion. Any waiter on the same session key can continue now.',
  })
}

function traceContextFromEnv():
  | {
      readonly baggage?: string
      readonly traceparent?: string
      readonly tracestate?: string
    }
  | undefined {
  const traceparent = process.env.TRACEPARENT
  const tracestate = process.env.TRACESTATE
  const baggage = process.env.BAGGAGE

  if (!traceparent && !tracestate && !baggage) {
    return undefined
  }

  return {
    ...(baggage ? { baggage } : {}),
    ...(traceparent ? { traceparent } : {}),
    ...(tracestate ? { tracestate } : {}),
  }
}

function emit(value: Record<string, unknown>): void {
  console.log(JSON.stringify(value))
}

function printUsage(): void {
  console.error(
    [
      'temporal-agent example',
      '',
      'Usage:',
      '  STATE_STREAM_URL=... SESSION_ID=... pnpm start -- wait',
      '  STATE_STREAM_URL=... SESSION_ID=... pnpm start -- resolve',
      '',
      'Environment:',
      '  STATE_STREAM_URL   Fireline durable state stream URL',
      '  SESSION_ID         canonical Fireline session id',
      '  OPENED_BY          optional resolver identity',
      '  CHANGE_WINDOW      optional human-readable window label',
      '  RESOLUTION_NOTE    optional resume note',
      '  TRACEPARENT        optional W3C traceparent for the completion envelope',
      '  TRACESTATE         optional W3C tracestate for the completion envelope',
      '  BAGGAGE            optional W3C baggage for the completion envelope',
    ].join('\n'),
  )
}

function requiredEnv(name: string): string {
  const value = process.env[name]
  if (!value) {
    throw new Error(`missing required environment variable: ${name}`)
  }
  return value
}
