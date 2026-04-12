export interface AgentSpawn {
  command: string
  args: string[]
}

export interface AgentTemplateRuntime {
  provider: string
  image?: string
  dockerfile?: string
  setup?: string
  env?: Record<string, string>
}

export interface AgentTemplate {
  id: string
  name: string
  spawn: AgentSpawn
  runtime: AgentTemplateRuntime
  env?: Record<string, string>
}

export interface SessionLog {
  timestamp: string
  type: string
  data: Record<string, unknown>
}

export interface PendingPermissionOption {
  optionId: string
  name: string
  kind: string
}

export interface PendingPermission {
  requestId: string
  toolCallId: string
  title: string
  kind?: string
  options: PendingPermissionOption[]
}

export interface FileSystemEntryGitInfo {
  branch: string
  origin?: string
}

export interface FileSystemEntry {
  path: string
  type: 'file' | 'directory' | 'symlink' | 'other'
  git?: FileSystemEntryGitInfo
}

export interface FileSystemSnapshot {
  root: string
  path?: string
  gitPath?: string
  entries: FileSystemEntry[]
  truncated: boolean
  maxEntries: number
}

export interface FilePreview {
  path: string
  content: string
  truncated: boolean
  maxChars: number
}

export interface PromptQueueItem {
  queueId: string
  text: string
  enqueuedAt: string
  position: number
}

export interface PromptQueueState {
  processing: boolean
  paused: boolean
  items: PromptQueueItem[]
  size: number
}

export interface Session {
  id: string
  agentName: string
  spawn: AgentSpawn
  startedAt: string
  lastUpdatedAt: string
  status: 'active' | 'killed'
  logs: SessionLog[]
  pendingPermission: PendingPermission | null
  fileSystem: FileSystemSnapshot | null
  promptQueue: PromptQueueState | null
  websocketUrl?: string
  runtime?: string
  cwd?: string
  title?: string
  acpUrl?: string
  stateStreamUrl?: string
}

export interface RegisterAgentTemplateBody {
  name: string
  spawn: AgentSpawn
  runtime?: AgentTemplateRuntime
  env?: Record<string, string>
}

export type PermissionResponseBody = { optionId: string } | { outcome: 'cancelled' }

export interface RuntimeInstance {
  name: string
  typeName: string
  status: 'running' | 'stopped' | 'paused'
  websocketUrl?: string
  acpUrl?: string
  stateStreamUrl?: string
}

export interface RuntimeInfo {
  typeName: string
  onlyOne: boolean
  instances: RuntimeInstance[]
}

export interface QueuedMessage {
  id: number
  sessionId: string | null
  text: string
  runtime: string
  agent: string
  agentTemplateId: string | null
  directory: string | null
  status: 'pending' | 'sent'
  createdAt: string
  sentAt: string | null
}

export interface SessionRuntimeInfo {
  hostUrl: string
  websocketUrl: string
  runtimeName: string
  runtimeMeta?: Record<string, unknown> | null
}

export interface PermissionRequestEvent {
  requestId: string
  toolCallId: string
  title: string
  kind?: string
  options: PendingPermissionOption[]
}

export const PendingPermissionSchema = {
  safeParse(value: unknown): { success: true; data: PendingPermission } | { success: false } {
    if (!isPendingPermission(value)) {
      return { success: false }
    }
    return { success: true, data: value }
  },
}

function isPendingPermission(value: unknown): value is PendingPermission {
  if (!isRecord(value)) {
    return false
  }
  return (
    typeof value.requestId === 'string' &&
    typeof value.toolCallId === 'string' &&
    typeof value.title === 'string' &&
    Array.isArray(value.options) &&
    value.options.every(isPendingPermissionOption)
  )
}

function isPendingPermissionOption(value: unknown): value is PendingPermissionOption {
  return (
    isRecord(value) &&
    typeof value.optionId === 'string' &&
    typeof value.name === 'string' &&
    typeof value.kind === 'string'
  )
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}
