import { createServer, type IncomingMessage, type ServerResponse } from 'node:http'
import { constants as fsConstants } from 'node:fs'
import { access } from 'node:fs/promises'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { createMemoryState } from '@chat-adapter/state-memory'
import { createTelegramAdapter } from '@chat-adapter/telegram'
import { Chat } from 'chat'

type ProbeStatus = {
  readonly checkedAt: string | null
  readonly detail: string | null
  readonly ok: boolean
}

type HealthSnapshot = {
  readonly adapterMode: 'polling' | 'webhook'
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

type TelegramIdentity = {
  readonly id: number
  readonly username?: string
}

type RuntimeStatus = {
  firelineHost: ProbeStatus
  startupMessage: {
    chatConfigured: boolean
    detail: string | null
    sentAt: string | null
  }
  stateStream: ProbeStatus
  telegramApi: ProbeStatus
}

const envFile = await loadEnvFile()
const config = readConfig()
const startedAt = new Date().toISOString()

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

const startupChatId =
  config.telegramChatId ?? (await getLatestTelegramChatId(config.botToken, config.telegramApiBaseUrl))

await bot.initialize()

logEvent('bridge_initialized', {
  adapterMode: telegram.runtimeMode,
  botUserName: `@${botUserName}`,
  callbackPath: config.callbackPath,
  envFileLoaded: envFile ?? 'none',
  firelineHostHealthz: config.firelineHealthUrl,
  stateStreamHealthz: config.stateStreamHealthUrl,
})

await refreshHealth()

if (startupChatId) {
  await sendStartupMessage(startupChatId, config.telegramChatId ? 'configured' : 'discovered')
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
          'Fireline Telegram bridge harness is running. Later beads add ACP routing, streaming, session mapping, and approval cards.',
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

function readConfig() {
  return {
    botToken: requireEnv('TELEGRAM_BOT_TOKEN'),
    callbackPath: normalizePath(process.env.BRIDGE_CALLBACK_WEBHOOK_PATH ?? '/telegram/callback'),
    firelineHealthUrl: toHealthUrl(process.env.FIRELINE_URL ?? 'http://127.0.0.1:4440'),
    port: parsePort(process.env.BRIDGE_PORT ?? '8787'),
    stateStreamHealthUrl: toHealthUrl(process.env.FIRELINE_STATE_STREAM_URL ?? 'http://127.0.0.1:7474'),
    telegramApiBaseUrl: trimTrailingSlash(
      process.env.TELEGRAM_API_BASE_URL ?? 'https://api.telegram.org',
    ),
    telegramChatId: process.env.TELEGRAM_CHAT_ID?.trim() || null,
  }
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
  const payload = await telegramApi<{ id: number; username?: string }>(
    botToken,
    telegramApiBaseUrl,
    'getMe',
  )
  return payload
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
      `Fireline Telegram bridge online.\n\nHealth: http://127.0.0.1:${config.port}/healthz\nRouting/streaming/approvals land in mono-thnc.6.3.3+.`,
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
  return status.telegramApi.ok && status.firelineHost.ok && status.stateStream.ok
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
