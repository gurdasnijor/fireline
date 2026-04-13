import fireline, { appendApprovalResolved } from '@fireline/client'
import { createMemoryState } from '@chat-adapter/state-memory'
import { createTelegramAdapter } from '@chat-adapter/telegram'
import {
  Actions,
  Button,
  Card,
  CardText,
  Chat,
  Field,
  Fields,
  type Message,
  type Thread,
} from 'chat'
import type { PermissionRow } from '@fireline/state'
import { constants as fsConstants } from 'node:fs'
import { access } from 'node:fs/promises'
import { createServer, type IncomingMessage, type ServerResponse } from 'node:http'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

type ProbeStatus = {
  readonly checkedAt: string | null
  readonly detail: string | null
  readonly ok: boolean
}

type ApprovalStatus = {
  readonly chatTarget: string | null
  readonly detail: string | null
  readonly knownPending: number
  readonly lastAnnouncedAt: string | null
  readonly ok: boolean
}

type HealthSnapshot = {
  readonly adapterMode: 'polling' | 'webhook'
  readonly approvalBridge: ApprovalStatus
  readonly botUserName: string
  readonly callbackPath: string
  readonly firelineHost: ProbeStatus
  readonly startedAt: string
  readonly startupMessage: {
    readonly chatConfigured: boolean
    readonly detail: string | null
    readonly sentAt: string | null
  }
  readonly stateStream: ProbeStatus
  readonly telegramApi: ProbeStatus
}

type PendingApproval = {
  readonly createdAt: number
  readonly requestId: string | number
  readonly sessionId: string
  readonly title: string | null
  readonly toolCallId: string | null
}

type TelegramIdentity = {
  readonly id: number
  readonly username?: string
}

type RuntimeStatus = {
  approvalBridge: {
    chatTarget: string | null
    detail: string | null
    knownPending: number
    lastAnnouncedAt: string | null
    ok: boolean
  }
  firelineHost: ProbeStatus
  startupMessage: {
    chatConfigured: boolean
    detail: string | null
    sentAt: string | null
  }
  stateStream: ProbeStatus
  telegramApi: ProbeStatus
}

type BridgeConfig = {
  readonly allowedUserIds: readonly string[]
  readonly botToken: string
  readonly callbackPath: string
  readonly firelineHealthUrl: string
  readonly port: number
  readonly stateStreamHealthUrl: string
  readonly stateStreamUrl: string
  readonly telegramApiBaseUrl: string
  readonly telegramChatId: string | null
}

const envFile = await loadEnvFile()
const config = readConfig()
const startedAt = new Date().toISOString()
const pendingApprovals = new Map<string, PendingApproval>()
const announcedApprovals = new Set<string>()

const identity = await getTelegramIdentity(config.botToken, config.telegramApiBaseUrl)
const botUserName = identity.username ?? process.env.TELEGRAM_BOT_USERNAME ?? 'fireline'

const telegram = createTelegramAdapter({
  apiBaseUrl: config.telegramApiBaseUrl,
  botToken: config.botToken,
  mode: 'polling',
  userName: botUserName,
})

const bot = new Chat({
  adapters: { telegram },
  state: createMemoryState(),
  userName: botUserName,
})

const status: RuntimeStatus = {
  approvalBridge: {
    chatTarget: null,
    detail: null,
    knownPending: 0,
    lastAnnouncedAt: null,
    ok: false,
  },
  firelineHost: { checkedAt: null, detail: null, ok: false } satisfies ProbeStatus,
  startupMessage: {
    chatConfigured: Boolean(config.telegramChatId),
    detail: null,
    sentAt: null,
  },
  stateStream: { checkedAt: null, detail: null, ok: false } satisfies ProbeStatus,
  telegramApi: {
    checkedAt: new Date().toISOString(),
    detail: `getMe ok for @${botUserName} (${identity.id})`,
    ok: true,
  } satisfies ProbeStatus,
}

registerChatHandlers()
await bot.initialize()

status.approvalBridge.ok = true
status.approvalBridge.detail = `watching ${config.stateStreamUrl}`

logEvent('bridge_initialized', {
  adapterMode: telegram.runtimeMode,
  botUserName: `@${botUserName}`,
  callbackPath: config.callbackPath,
  envFileLoaded: envFile ?? 'none',
  firelineHostHealthz: config.firelineHealthUrl,
  stateStreamUrl: config.stateStreamUrl,
})

const db = await fireline.db({ stateStreamUrl: config.stateStreamUrl })
const approvalSubscription = db.permissions.subscribe((rows) => {
  void syncPendingApprovals(rows)
})

await refreshHealth()

rememberChatTarget(
  config.telegramChatId ?? (await getLatestTelegramChatId(config.botToken, config.telegramApiBaseUrl)),
)

if (status.approvalBridge.chatTarget) {
  await sendStartupMessage(
    status.approvalBridge.chatTarget,
    config.telegramChatId ? 'configured' : 'discovered',
  )
} else if (!config.telegramChatId) {
  status.startupMessage.detail =
    'no TELEGRAM_CHAT_ID configured and getUpdates returned no prior chat to bootstrap'
}

const healthTimer = setInterval(() => {
  void refreshHealth()
}, 10_000)

const server = createServer(async (req, res) => {
  try {
    const url = requestUrl(req)

    if (req.method === 'GET' && url.pathname === '/healthz') {
      return writeJson(res, isHealthy() ? 200 : 503, snapshot())
    }

    if (req.method === 'GET' && url.pathname === '/') {
      return writeJson(res, 200, {
        callbackPath: config.callbackPath,
        healthz: '/healthz',
        message:
          'Fireline Telegram approval bridge is running. DM the bot `help` for status commands and approval cards.',
      })
    }

    if (req.method === 'POST' && url.pathname === config.callbackPath) {
      const response = await bot.webhooks.telegram(await toWebRequest(req, url))
      await writeWebResponse(res, response)
      return
    }

    res.writeHead(404, { 'content-type': 'application/json; charset=utf-8' })
    res.end(JSON.stringify({ error: 'not_found' }))
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    res.writeHead(500, { 'content-type': 'application/json; charset=utf-8' })
    res.end(JSON.stringify({ error: 'bridge_error', message }))
  }
})

await new Promise<void>((resolveListen) => {
  server.listen(config.port, '0.0.0.0', resolveListen)
})

logEvent('bridge_listening', {
  adapterMode: telegram.runtimeMode,
  chatTarget: status.approvalBridge.chatTarget,
  port: config.port,
  startupMessage: status.startupMessage.sentAt ? 'sent' : 'skipped',
})

let shuttingDown = false

for (const signal of ['SIGINT', 'SIGTERM'] as const) {
  process.on(signal, () => {
    if (shuttingDown) {
      return
    }
    shuttingDown = true
    void shutdown(signal)
  })
}

async function shutdown(signal: string) {
  clearInterval(healthTimer)
  approvalSubscription.unsubscribe()
  db.close()
  logEvent('bridge_shutdown_start', { signal })
  await bot.shutdown().catch((error: unknown) => {
    logEvent('bridge_shutdown_error', {
      error: error instanceof Error ? error.message : String(error),
    })
  })
  await new Promise<void>((resolveClose) => server.close(() => resolveClose()))
  logEvent('bridge_shutdown_complete', { signal })
  process.exit(0)
}

function registerChatHandlers() {
  bot.onDirectMessage(async (thread, message) => {
    await thread.subscribe()
    await handleOperatorMessage(thread, message)
  })

  bot.onNewMention(async (thread, message) => {
    await thread.subscribe()
    await handleOperatorMessage(thread, message)
  })

  bot.onSubscribedMessage(async (thread, message) => {
    await handleOperatorMessage(thread, message)
  })

  bot.onAction(['approve', 'deny'], async (event) => {
    if (!isAllowedUser(event.user.userId)) {
      if (event.thread) {
        await event.thread.post(
          'This bridge is restricted to the Telegram operator IDs in TELEGRAM_ALLOWED_USER_IDS.',
        )
      }
      return
    }

    rememberChatTarget(event.thread?.channelId ?? status.approvalBridge.chatTarget)

    const key = event.value?.trim()
    if (!key) {
      if (event.thread) {
        await event.thread.post('Approval action arrived without a request id payload.')
      }
      return
    }

    const approval = pendingApprovals.get(key)
    if (!approval) {
      if (event.thread) {
        await event.thread.post(
          'That approval is no longer pending. Send `pending` to see the current queue.',
        )
      }
      return
    }

    const allow = event.actionId === 'approve'
    await resolveApproval(approval, allow)
    if (event.thread) {
      await event.thread.post(formatResolutionMessage(approval, allow, event.user.userName))
    }
  })
}

async function handleOperatorMessage(thread: Thread, message: Message) {
  rememberChatTarget(thread.channelId)

  if (!isAllowedUser(message.author.userId)) {
    await thread.post(
      'This bridge is restricted to the Telegram operator IDs in TELEGRAM_ALLOWED_USER_IDS.',
    )
    return
  }

  const command = parseCommand(message.text)

  if (command.name === 'help') {
    await thread.post(formatHelpMessage())
    return
  }

  if (command.name === 'status') {
    await refreshHealth()
    await thread.post(formatStatusMessage())
    return
  }

  if (command.name === 'pending') {
    await thread.post(formatPendingMessage())
    return
  }

  if (command.name === 'approve' || command.name === 'deny') {
    if (command.args.length < 2) {
      await thread.post(
        `Use \`${command.name} <session-id> <request-id>\`, or tap the card buttons on a pending approval.`,
      )
      return
    }

    const approval = pendingApprovals.get(approvalKey(command.args[0], command.args[1]))
    if (!approval) {
      await thread.post('No pending approval matches that session id and request id.')
      return
    }

    const allow = command.name === 'approve'
    await resolveApproval(approval, allow)
    await thread.post(formatResolutionMessage(approval, allow, message.author.userName))
    return
  }

  await thread.post(
    [
      `I bridge Fireline approvals into Telegram.`,
      '',
      `Send \`status\` to probe the host and stream, or \`pending\` to list queued approvals.`,
      `When a run pauses for approval, I'll post an inline Approve / Deny card here automatically.`,
    ].join('\n'),
  )
}

async function resolveApproval(approval: PendingApproval, allow: boolean) {
  await appendApprovalResolved({
    allow,
    requestId: approval.requestId,
    resolvedBy: 'telegram-bridge',
    sessionId: approval.sessionId,
    streamUrl: config.stateStreamUrl,
  })
}

async function syncPendingApprovals(rows: ReadonlyArray<PermissionRow>) {
  const nextPending = new Map<string, PendingApproval>()

  for (const row of rows) {
    if (row.state !== 'pending' || row.requestId === null) {
      continue
    }

    const approval: PendingApproval = {
      createdAt: row.createdAt,
      requestId: row.requestId,
      sessionId: row.sessionId,
      title: row.title ?? null,
      toolCallId: row.toolCallId ?? null,
    }
    nextPending.set(approvalKey(approval.sessionId, approval.requestId), approval)
  }

  pendingApprovals.clear()
  for (const [key, approval] of nextPending) {
    pendingApprovals.set(key, approval)
  }

  status.approvalBridge.knownPending = pendingApprovals.size

  if (!status.approvalBridge.chatTarget) {
    status.approvalBridge.detail =
      'waiting for TELEGRAM_CHAT_ID or the first operator DM before posting approval cards'
    return
  }

  for (const [key, approval] of pendingApprovals) {
    if (announcedApprovals.has(key)) {
      continue
    }
    announcedApprovals.add(key)
    status.approvalBridge.lastAnnouncedAt = new Date().toISOString()
    void postPendingApprovalCard(status.approvalBridge.chatTarget, approval)
  }
}

async function postPendingApprovalCard(chatId: string, approval: PendingApproval) {
  const detail = approval.title ?? 'No approval reason was recorded.'
  await telegram.postChannelMessage(
    chatId,
    Card({
      subtitle: `session ${shortId(approval.sessionId)} · request ${approval.requestId}`,
      title: 'Approval needed',
      children: [
        CardText('A Fireline run is paused on a gated tool call.'),
        Fields([
          Field({ label: 'session', value: shortId(approval.sessionId) }),
          Field({ label: 'request', value: String(approval.requestId) }),
          Field({ label: 'tool', value: approval.toolCallId ?? 'unknown' }),
        ]),
        CardText(detail),
        Actions([
          Button({
            id: 'approve',
            label: 'Approve',
            style: 'primary',
            value: approvalKey(approval.sessionId, approval.requestId),
          }),
          Button({
            id: 'deny',
            label: 'Deny',
            style: 'danger',
            value: approvalKey(approval.sessionId, approval.requestId),
          }),
        ]),
      ],
    }),
  )
}

function readConfig(): BridgeConfig {
  const stateStreamUrl = readStateStreamUrl()

  return {
    allowedUserIds: parseList(process.env.TELEGRAM_ALLOWED_USER_IDS),
    botToken: requireEnv('TELEGRAM_BOT_TOKEN'),
    callbackPath: normalizePath(process.env.BRIDGE_CALLBACK_WEBHOOK_PATH ?? '/telegram/callback'),
    firelineHealthUrl: toHealthUrl(process.env.FIRELINE_URL ?? 'http://127.0.0.1:4440'),
    port: parsePort(process.env.BRIDGE_PORT ?? '8787'),
    stateStreamHealthUrl: toHealthUrl(stateStreamUrl),
    stateStreamUrl,
    telegramApiBaseUrl: trimTrailingSlash(
      process.env.TELEGRAM_API_BASE_URL ?? 'https://api.telegram.org',
    ),
    telegramChatId: process.env.TELEGRAM_CHAT_ID?.trim() || null,
  }
}

function readStateStreamUrl(): string {
  return (
    process.env.FIRELINE_STATE_STREAM_URL?.trim() ||
    process.env.FIRELINE_STREAM_URL?.trim() ||
    'http://127.0.0.1:7474/streams/state/default'
  )
}

async function loadEnvFile(): Promise<string | null> {
  const explicitPath = process.env.BRIDGE_ENV_FILE?.trim()
  const localPath = resolve(
    fileURLToPath(new URL('../../deploy/telegram/bridge.env', import.meta.url)),
  )
  const candidates = explicitPath ? [explicitPath] : [localPath]

  for (const candidate of candidates) {
    if (!(await fileExists(candidate))) {
      continue
    }
    process.loadEnvFile(candidate)
    return candidate
  }

  return null
}

async function fileExists(path: string): Promise<boolean> {
  try {
    await access(path, fsConstants.R_OK)
    return true
  } catch {
    return false
  }
}

function requestUrl(req: IncomingMessage): URL {
  return new URL(req.url ?? '/', `http://${req.headers.host ?? '127.0.0.1'}`)
}

async function toWebRequest(req: IncomingMessage, url: URL): Promise<Request> {
  const headers = new Headers()
  for (const [key, value] of Object.entries(req.headers)) {
    if (value === undefined) {
      continue
    }
    if (Array.isArray(value)) {
      for (const entry of value) {
        headers.append(key, entry)
      }
      continue
    }
    headers.set(key, value)
  }

  const body =
    req.method === 'GET' || req.method === 'HEAD'
      ? undefined
      : new Uint8Array(await readBody(req))

  return new Request(url, {
    body,
    headers,
    method: req.method ?? 'GET',
  })
}

async function writeWebResponse(res: ServerResponse, response: Response) {
  const headers: Record<string, string> = {}
  response.headers.forEach((value, key) => {
    headers[key] = value
  })
  res.writeHead(response.status, headers)
  const body = new Uint8Array(await response.arrayBuffer())
  res.end(body)
}

async function readBody(req: IncomingMessage): Promise<Buffer> {
  const chunks: Buffer[] = []
  for await (const chunk of req) {
    chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk))
  }
  return Buffer.concat(chunks)
}

async function refreshHealth() {
  const [telegramApi, firelineHost, stateStream] = await Promise.all([
    probeTelegram(),
    probeHttp(config.firelineHealthUrl),
    probeHttp(config.stateStreamHealthUrl),
  ])

  status.telegramApi = telegramApi
  status.firelineHost = firelineHost
  status.stateStream = stateStream
}

async function probeTelegram(): Promise<ProbeStatus> {
  const checkedAt = new Date().toISOString()

  try {
    const identity = await getTelegramIdentity(config.botToken, config.telegramApiBaseUrl)
    return {
      checkedAt,
      detail: `getMe ok for @${identity.username ?? botUserName} (${identity.id})`,
      ok: true,
    }
  } catch (error) {
    return {
      checkedAt,
      detail: error instanceof Error ? error.message : String(error),
      ok: false,
    }
  }
}

async function probeHttp(url: string): Promise<ProbeStatus> {
  const checkedAt = new Date().toISOString()
  const controller = new AbortController()
  const timeout = setTimeout(() => controller.abort(), 3_000)

  try {
    const response = await fetch(url, { signal: controller.signal })
    return {
      checkedAt,
      detail: `${response.status} ${response.statusText}`.trim(),
      ok: response.ok,
    }
  } catch (error) {
    return {
      checkedAt,
      detail: error instanceof Error ? error.message : String(error),
      ok: false,
    }
  } finally {
    clearTimeout(timeout)
  }
}

async function getTelegramIdentity(
  botToken: string,
  telegramApiBaseUrl: string,
): Promise<TelegramIdentity> {
  return telegramApi<{ id: number; username?: string }>(
    botToken,
    telegramApiBaseUrl,
    'getMe',
  )
}

async function getLatestTelegramChatId(
  botToken: string,
  telegramApiBaseUrl: string,
): Promise<string | null> {
  let updates:
    | {
        callback_query?: { message?: { chat?: { id?: number | string } } }
        channel_post?: { chat?: { id?: number | string } }
        edited_channel_post?: { chat?: { id?: number | string } }
        edited_message?: { chat?: { id?: number | string } }
        message?: { chat?: { id?: number | string } }
        update_id: number
      }[]
    | null = null

  try {
    updates = await telegramApi<
      {
        callback_query?: { message?: { chat?: { id?: number | string } } }
        channel_post?: { chat?: { id?: number | string } }
        edited_channel_post?: { chat?: { id?: number | string } }
        edited_message?: { chat?: { id?: number | string } }
        message?: { chat?: { id?: number | string } }
        update_id: number
      }[]
    >(botToken, telegramApiBaseUrl, 'getUpdates')
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    if (message.includes('409')) {
      return null
    }
    throw error
  }

  for (const update of [...updates].reverse()) {
    const chatId =
      update.message?.chat?.id ??
      update.edited_message?.chat?.id ??
      update.channel_post?.chat?.id ??
      update.edited_channel_post?.chat?.id ??
      update.callback_query?.message?.chat?.id
    if (chatId !== undefined) {
      return String(chatId)
    }
  }

  return null
}

async function telegramApi<T>(
  botToken: string,
  telegramApiBaseUrl: string,
  method: string,
): Promise<T> {
  const response = await fetch(`${telegramApiBaseUrl}/bot${botToken}/${method}`)
  if (!response.ok) {
    throw new Error(`Telegram ${method} failed: ${response.status} ${response.statusText}`)
  }

  const payload = (await response.json()) as {
    ok?: boolean
    description?: string
    result?: T
  }

  if (!payload.ok || !payload.result) {
    throw new Error(payload.description ?? `Telegram ${method} returned no result`)
  }

  return payload.result
}

async function sendStartupMessage(chatId: string, source: 'configured' | 'discovered') {
  try {
    await telegram.postChannelMessage(
      chatId,
      [
        'Fireline Telegram approval bridge online.',
        '',
        'I watch the durable permission stream and post inline Approve / Deny cards here.',
        'Commands: status, pending, help',
        `Watching: ${config.stateStreamUrl}`,
        `Health: http://127.0.0.1:${config.port}/healthz`,
      ].join('\n'),
    )
    status.startupMessage.sentAt = new Date().toISOString()
    status.startupMessage.detail = `startup message posted to ${source} chat`
    logEvent('startup_message_posted', {
      source,
      sentAt: status.startupMessage.sentAt,
    })
  } catch (error) {
    status.startupMessage.detail = error instanceof Error ? error.message : String(error)
    logEvent('startup_message_failed', {
      error: status.startupMessage.detail,
    })
  }
}

function snapshot(): HealthSnapshot {
  return {
    adapterMode: telegram.runtimeMode,
    approvalBridge: status.approvalBridge,
    botUserName: `@${botUserName}`,
    callbackPath: config.callbackPath,
    firelineHost: status.firelineHost,
    startedAt,
    startupMessage: status.startupMessage,
    stateStream: status.stateStream,
    telegramApi: status.telegramApi,
  }
}

function isHealthy(): boolean {
  return (
    status.approvalBridge.ok &&
    status.telegramApi.ok &&
    status.firelineHost.ok &&
    status.stateStream.ok
  )
}

function writeJson(res: ServerResponse, statusCode: number, body: unknown) {
  res.writeHead(statusCode, { 'content-type': 'application/json; charset=utf-8' })
  res.end(JSON.stringify(body, null, 2))
}

function requireEnv(name: string): string {
  const value = process.env[name]?.trim()
  if (!value) {
    throw new Error(`Missing required environment variable ${name}`)
  }
  return value
}

function normalizePath(path: string): string {
  if (path === '/') {
    return path
  }
  return path.startsWith('/') ? path.replace(/\/+$/, '') : `/${path.replace(/\/+$/, '')}`
}

function parsePort(value: string): number {
  const parsed = Number.parseInt(value, 10)
  if (!Number.isFinite(parsed) || parsed <= 0) {
    throw new Error(`Invalid BRIDGE_PORT '${value}'`)
  }
  return parsed
}

function trimTrailingSlash(value: string): string {
  return value.replace(/\/+$/, '')
}

function toHealthUrl(baseUrl: string): string {
  return new URL('/healthz', baseUrl).toString()
}

function logEvent(event: string, fields: Record<string, unknown>) {
  console.log(JSON.stringify({ event, ...fields }))
}

function parseList(value: string | undefined): string[] {
  return (value ?? '')
    .split(',')
    .map((entry) => entry.trim())
    .filter(Boolean)
}

function rememberChatTarget(chatId: string | null | undefined) {
  if (!chatId) {
    return
  }
  status.approvalBridge.chatTarget = chatId
}

function isAllowedUser(userId: string): boolean {
  return config.allowedUserIds.length === 0 || config.allowedUserIds.includes(String(userId))
}

function parseCommand(text: string): { readonly args: readonly string[]; readonly name: string } {
  const normalized = stripBotMention(text).trim()
  if (!normalized) {
    return { args: [], name: 'help' }
  }

  const tokens = normalized.split(/\s+/)
  const name = tokens[0]?.replace(/^\//, '').toLowerCase() || 'help'
  return { args: tokens.slice(1), name }
}

function stripBotMention(text: string): string {
  return text.replace(new RegExp(`^@${escapeRegExp(botUserName)}[,:\\s-]*`, 'i'), '')
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

function approvalKey(sessionId: string, requestId: string | number): string {
  return `${sessionId}::${String(requestId)}`
}

function shortId(value: string, width = 8): string {
  return value.length <= width ? value : value.slice(0, width)
}

function formatHelpMessage(): string {
  return [
    'I bridge Fireline approvals into Telegram.',
    '',
    'Commands:',
    '- status — probe the Fireline host and durable stream',
    '- pending — list queued approvals I can still resolve',
    '- approve <session-id> <request-id> — text fallback if you do not want to tap buttons',
    '- deny <session-id> <request-id> — deny the pending approval',
  ].join('\n')
}

function formatStatusMessage(): string {
  return [
    `bot: @${botUserName} (${telegram.runtimeMode})`,
    `host: ${status.firelineHost.ok ? 'ok' : 'down'}${formatDetail(status.firelineHost.detail)}`,
    `stream: ${status.stateStream.ok ? 'ok' : 'down'}${formatDetail(status.stateStream.detail)}`,
    `pending approvals: ${status.approvalBridge.knownPending}`,
    `chat target: ${status.approvalBridge.chatTarget ?? 'waiting for first DM'}`,
  ].join('\n')
}

function formatPendingMessage(): string {
  if (pendingApprovals.size === 0) {
    return 'No approvals are pending right now.'
  }

  const lines = ['Pending approvals:']
  for (const approval of [...pendingApprovals.values()].sort((left, right) => left.createdAt - right.createdAt)) {
    lines.push(
      `- ${shortId(approval.sessionId)} / ${approval.requestId} · ${approval.toolCallId ?? 'tool'} · ${approval.title ?? 'No reason recorded'}`,
    )
  }
  return lines.join('\n')
}

function formatResolutionMessage(
  approval: PendingApproval,
  allow: boolean,
  actor: string,
): string {
  return `${allow ? 'Approved' : 'Denied'} ${shortId(approval.sessionId)} / ${approval.requestId} as ${actor}.`
}

function formatDetail(detail: string | null): string {
  return detail ? ` (${detail})` : ''
}
