import { Sandbox } from '../../../packages/client/src/sandbox.ts'
import { SandboxAdmin } from '../../../packages/client/src/admin.ts'
import type {
  AgentTemplate,
  FilePreview,
  FileSystemSnapshot,
  PermissionResponseBody,
  QueuedMessage,
  RegisterAgentTemplateBody,
  RuntimeInfo,
  RuntimeInstance,
  Session,
} from './fireline-types.js'

export type FlamecastClientOptions = {
  baseUrl: string | URL
  fetch?: typeof fetch
}

export type GitBranch = {
  name: string
  sha: string
  current: boolean
  remote: boolean
}

export type GitBranchesResponse = {
  branches: GitBranch[]
}

export type GitWorktree = {
  path: string
  sha?: string
  branch?: string
  bare?: boolean
  detached?: boolean
}

export type GitWorktreesResponse = {
  worktrees: GitWorktree[]
}

export type GitWorktreeCreateResponse = {
  path: string
  message: string
}

export type FlamecastSettings = {
  autoApprovePermissions: boolean
}

export type SessionSettings = {
  autoApprovePermissions: boolean
}

export type UpdateAgentTemplateBody = {
  name?: string
  spawn?: AgentTemplate['spawn']
  runtime?: Partial<AgentTemplate['runtime']>
  env?: Record<string, string>
}

export interface FlamecastClient {
  readonly sandbox: Sandbox
  readonly admin: SandboxAdmin
  readonly rpc: {
    runtimes: {
      ':instanceName': {
        fs: {
          commands: {
            $get(input: {
              param: { instanceName: string }
              query?: { path?: string }
            }): Promise<Response>
          }
        }
      }
    }
    agents: {
      ':agentId': {
        commands: {
          $get(input: {
            param: { agentId: string }
          }): Promise<Response>
        }
      }
    }
  }
  fetchFirelineConfig(): Promise<{
    firelineUrl: string
    stateStreamUrl: string
    workspaceRoot: string
  }>
  fetchSettings(): Promise<FlamecastSettings>
  updateSettings(patch: Partial<FlamecastSettings>): Promise<FlamecastSettings>
  fetchAgentTemplates(): Promise<AgentTemplate[]>
  registerAgentTemplate(body: RegisterAgentTemplateBody): Promise<AgentTemplate>
  updateAgentTemplate(id: string, body: UpdateAgentTemplateBody): Promise<AgentTemplate>
  fetchSessions(): Promise<Session[]>
  fetchSession(id: string, opts?: { includeFileSystem?: boolean; showAllFiles?: boolean }): Promise<Session>
  fetchSessionFilePreview(id: string, path: string): Promise<FilePreview>
  fetchSessionFileSystem(
    id: string,
    opts?: { showAllFiles?: boolean; path?: string },
  ): Promise<FileSystemSnapshot>
  createSession(body: {
    sessionId?: string
    cwd?: string
    agentTemplateId?: string
    runtimeInstance?: string
    name?: string
  }): Promise<Session>
  terminateSession(id: string): Promise<void>
  promptSession(id: string, text: string): Promise<Record<string, unknown>>
  fetchSessionStatus(id: string): Promise<{ processing: boolean; pendingPermission: boolean }>
  resolvePermission(
    sessionId: string,
    requestId: string,
    body: PermissionResponseBody,
  ): Promise<Record<string, unknown>>
  fetchSessionSettings(id: string): Promise<SessionSettings>
  updateSessionSettings(id: string, patch: Partial<SessionSettings>): Promise<SessionSettings>
  listMessageQueue(): Promise<QueuedMessage[]>
  enqueueMessage(
    message: Omit<QueuedMessage, 'id' | 'createdAt' | 'sentAt' | 'status'>,
  ): Promise<QueuedMessage>
  sendQueuedMessage(id: number): Promise<Record<string, unknown>>
  removeQueuedMessage(id: number): Promise<void>
  clearMessageQueue(): Promise<void>
  fetchRuntimes(): Promise<RuntimeInfo[]>
  fetchRuntimeFilePreview(instanceName: string, path: string): Promise<FilePreview>
  fetchRuntimeFileSystem(
    instanceName: string,
    opts?: { showAllFiles?: boolean; path?: string },
  ): Promise<FileSystemSnapshot>
  startRuntime(typeName: string, name?: string): Promise<RuntimeInstance>
  stopRuntime(instanceName: string): Promise<void>
  pauseRuntime(instanceName: string): Promise<void>
  deleteRuntime(instanceName: string): Promise<void>
  fetchRuntimeGitBranches(
    instanceName: string,
    opts?: { path?: string },
  ): Promise<GitBranchesResponse>
  fetchRuntimeGitWorktrees(
    instanceName: string,
    opts?: { path?: string },
  ): Promise<GitWorktreesResponse>
  createRuntimeGitWorktree(
    instanceName: string,
    body: {
      name: string
      path?: string
      branch?: string
      newBranch?: boolean
      startPoint?: string
    },
  ): Promise<GitWorktreeCreateResponse>
}

export function createFlamecastClient(options: FlamecastClientOptions): FlamecastClient {
  const baseUrl = normalizeBaseUrl(options.baseUrl)
  const fetchImpl = options.fetch ?? fetch
  const sandbox = new Sandbox({ serverUrl: baseUrl })
  const admin = new SandboxAdmin({ serverUrl: baseUrl })

  const request = async <T>(path: string, init: RequestInit = {}): Promise<T> => {
    const response = await fetchImpl(`${baseUrl}${path}`, {
      headers: {
        accept: 'application/json',
        ...(init.body ? { 'content-type': 'application/json' } : {}),
        ...(init.headers ?? {}),
      },
      ...init,
    })
    if (!response.ok) {
      throw new Error(`request failed (${response.status}) for ${path}`)
    }
    if (response.status === 204) {
      return undefined as T
    }
    return (await response.json()) as T
  }

  const raw = (path: string, init: RequestInit = {}) =>
    fetchImpl(`${baseUrl}${path}`, {
      headers: {
        accept: 'application/json',
        ...(init.body ? { 'content-type': 'application/json' } : {}),
        ...(init.headers ?? {}),
      },
      ...init,
    })

  return {
    sandbox,
    admin,
    rpc: {
      runtimes: {
        ':instanceName': {
          fs: {
            commands: {
              $get({ param, query }) {
                const search = new URLSearchParams()
                if (query?.path) {
                  search.set('path', query.path)
                }
                const suffix = search.toString()
                return raw(`/api/runtimes/${encodeURIComponent(param.instanceName)}/fs/commands${suffix ? `?${suffix}` : ''}`)
              },
            },
          },
        },
      },
      agents: {
        ':agentId': {
          commands: {
            $get({ param }) {
              return raw(`/api/agents/${encodeURIComponent(param.agentId)}/commands`)
            },
          },
        },
      },
    },
    fetchFirelineConfig() {
      return request('/api/fireline-config')
    },
    fetchSettings() {
      return request('/api/settings')
    },
    updateSettings(patch) {
      return request('/api/settings', { method: 'PATCH', body: JSON.stringify(patch) })
    },
    fetchAgentTemplates() {
      return request('/api/agent-templates')
    },
    registerAgentTemplate(body) {
      return request('/api/agent-templates', { method: 'POST', body: JSON.stringify(body) })
    },
    updateAgentTemplate(id, body) {
      return request(`/api/agent-templates/${encodeURIComponent(id)}`, {
        method: 'PUT',
        body: JSON.stringify(body),
      })
    },
    fetchSessions() {
      return request('/api/agents')
    },
    fetchSession(id, opts = {}) {
      const search = new URLSearchParams()
      if (opts.includeFileSystem) {
        search.set('includeFileSystem', 'true')
      }
      if (opts.showAllFiles) {
        search.set('showAllFiles', 'true')
      }
      const suffix = search.toString()
      return request(`/api/agents/${encodeURIComponent(id)}${suffix ? `?${suffix}` : ''}`)
    },
    fetchSessionFilePreview(id, path) {
      return request(
        `/api/agents/${encodeURIComponent(id)}/files?${new URLSearchParams({ path }).toString()}`,
      )
    },
    fetchSessionFileSystem(id, opts = {}) {
      const search = new URLSearchParams()
      if (opts.showAllFiles) {
        search.set('showAllFiles', 'true')
      }
      if (opts.path) {
        search.set('path', opts.path)
      }
      return request(`/api/agents/${encodeURIComponent(id)}/fs/snapshot?${search.toString()}`)
    },
    createSession(body) {
      return request('/api/agents', { method: 'POST', body: JSON.stringify(body) })
    },
    async terminateSession(id) {
      await request(`/api/agents/${encodeURIComponent(id)}`, { method: 'DELETE' })
    },
    promptSession(id, text) {
      return request(`/api/agents/${encodeURIComponent(id)}/prompts`, {
        method: 'POST',
        body: JSON.stringify({ text }),
      })
    },
    fetchSessionStatus(id) {
      return request(`/api/agents/${encodeURIComponent(id)}/status`)
    },
    resolvePermission(sessionId, requestId, body) {
      return request(
        `/api/agents/${encodeURIComponent(sessionId)}/permissions/${encodeURIComponent(requestId)}`,
        {
          method: 'POST',
          body: JSON.stringify(body),
        },
      )
    },
    fetchSessionSettings(id) {
      return request(`/api/agents/${encodeURIComponent(id)}/settings`)
    },
    updateSessionSettings(id, patch) {
      return request(`/api/agents/${encodeURIComponent(id)}/settings`, {
        method: 'PATCH',
        body: JSON.stringify(patch),
      })
    },
    listMessageQueue() {
      return request('/api/message-queue')
    },
    enqueueMessage(message) {
      return request('/api/message-queue', { method: 'POST', body: JSON.stringify(message) })
    },
    sendQueuedMessage(id) {
      return request(`/api/message-queue/${id}/send`, { method: 'POST' })
    },
    async removeQueuedMessage(id) {
      await request(`/api/message-queue/${id}`, { method: 'DELETE' })
    },
    async clearMessageQueue() {
      await request('/api/message-queue', { method: 'DELETE' })
    },
    fetchRuntimes() {
      return request('/api/runtimes')
    },
    fetchRuntimeFilePreview(instanceName, path) {
      return request(
        `/api/runtimes/${encodeURIComponent(instanceName)}/files?${new URLSearchParams({ path }).toString()}`,
      )
    },
    fetchRuntimeFileSystem(instanceName, opts = {}) {
      const search = new URLSearchParams()
      if (opts.showAllFiles) {
        search.set('showAllFiles', 'true')
      }
      if (opts.path) {
        search.set('path', opts.path)
      }
      return request(`/api/runtimes/${encodeURIComponent(instanceName)}/fs/snapshot?${search.toString()}`)
    },
    startRuntime(typeName, name) {
      return request(`/api/runtimes/${encodeURIComponent(typeName)}/start`, {
        method: 'POST',
        body: JSON.stringify(name ? { name } : {}),
      })
    },
    async stopRuntime(instanceName) {
      await request(`/api/runtimes/${encodeURIComponent(instanceName)}/stop`, { method: 'POST' })
    },
    async pauseRuntime(instanceName) {
      await request(`/api/runtimes/${encodeURIComponent(instanceName)}/pause`, { method: 'POST' })
    },
    async deleteRuntime(instanceName) {
      await request(`/api/runtimes/${encodeURIComponent(instanceName)}`, { method: 'DELETE' })
    },
    fetchRuntimeGitBranches(instanceName, opts = {}) {
      const search = new URLSearchParams()
      if (opts.path) {
        search.set('path', opts.path)
      }
      const suffix = search.toString()
      return request(`/api/runtimes/${encodeURIComponent(instanceName)}/fs/git/branches${suffix ? `?${suffix}` : ''}`)
    },
    fetchRuntimeGitWorktrees(instanceName, opts = {}) {
      const search = new URLSearchParams()
      if (opts.path) {
        search.set('path', opts.path)
      }
      const suffix = search.toString()
      return request(`/api/runtimes/${encodeURIComponent(instanceName)}/fs/git/worktrees${suffix ? `?${suffix}` : ''}`)
    },
    createRuntimeGitWorktree(instanceName, body) {
      return request(`/api/runtimes/${encodeURIComponent(instanceName)}/fs/git/worktrees`, {
        method: 'POST',
        body: JSON.stringify(body),
      })
    },
  }
}

function normalizeBaseUrl(baseUrl: string | URL): string {
  return String(baseUrl).replace(/\/$/, '')
}

export type { AgentTemplate, FilePreview, FileSystemSnapshot, QueuedMessage, RuntimeInfo, RuntimeInstance, Session }
