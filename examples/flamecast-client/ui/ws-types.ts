import type { PermissionResponseBody } from './fireline-types.js'

export type WsChannelControlMessage =
  | { action: 'subscribe'; channel: string; since?: number }
  | { action: 'unsubscribe'; channel: string }
  | { action: 'prompt'; sessionId: string; text: string }
  | { action: 'permission.respond'; sessionId: string; requestId: string; body: PermissionResponseBody }
  | { action: 'cancel'; sessionId: string; queueId?: string }
  | { action: 'terminate'; sessionId: string }
  | { action: 'terminal.input'; terminalId: string; data: string }
  | { action: 'terminal.resize'; terminalId: string; cols: number; rows: number }
  | { action: 'terminal.create'; data?: string }
  | { action: 'terminal.kill'; terminalId: string }

export type WsChannelServerMessage =
  | { type: 'connected' }
  | { type: 'subscribed'; channel: string }
  | { type: 'unsubscribed'; channel: string }
  | { type: 'pong' }
  | { type: 'error'; message: string }
  | {
      type: 'event'
      channel: string
      seq: number
      event: {
        type: string
        timestamp: string
        data: Record<string, unknown>
      }
    }
