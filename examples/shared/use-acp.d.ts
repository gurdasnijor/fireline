declare module 'use-acp' {
  import type {
    Agent,
    AgentCapabilities,
    AvailableCommand,
    McpServer,
    RequestPermissionRequest,
    RequestPermissionResponse,
    SessionModeState,
    SessionNotification,
  } from '@agentclientprotocol/sdk'

  export interface UseAcpClientOptions {
    readonly wsUrl: string
    readonly autoConnect?: boolean
    readonly reconnectAttempts?: number
    readonly reconnectDelay?: number
    readonly clientOptions?: Record<string, unknown>
    readonly initialSessionId?: string | null
    readonly sessionParams?: {
      readonly cwd?: string
      readonly mcpServers?: McpServer[]
    }
  }

  export interface ConnectionState {
    readonly status: 'disconnected' | 'connecting' | 'connected' | 'error'
    readonly error?: string
    readonly url?: string
  }

  export type NotificationEvent =
    | {
        readonly id: string
        readonly timestamp: number
        readonly type: 'session_notification'
        readonly data: SessionNotification
      }
    | {
        readonly id: string
        readonly timestamp: number
        readonly type: 'connection_change'
        readonly data: ConnectionState
      }
    | {
        readonly id: string
        readonly timestamp: number
        readonly type: 'error'
        readonly data: Error
      }

  export interface IdentifiedPermissionRequest extends RequestPermissionRequest {
    readonly deferredId: string
  }

  export interface UseAcpClientReturn {
    readonly connect: () => Promise<void>
    readonly disconnect: () => void
    readonly connectionState: ConnectionState
    readonly activeSessionId: string | null
    readonly setActiveSessionId: (sessionId: string | null) => void
    readonly notifications: NotificationEvent[]
    readonly clearNotifications: (sessionId?: string) => void
    readonly isSessionLoading: boolean
    readonly pendingPermission: IdentifiedPermissionRequest | null
    readonly resolvePermission: (response: RequestPermissionResponse) => void
    readonly rejectPermission: (error: Error) => void
    readonly agent: Agent | null
    readonly agentCapabilities: AgentCapabilities | null
    readonly availableCommands: AvailableCommand[]
    readonly sessionMode: SessionModeState | null | undefined
  }

  export function useAcpClient(options: UseAcpClientOptions): UseAcpClientReturn
}
