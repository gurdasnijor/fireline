#!/usr/bin/env node

import { randomUUID } from 'node:crypto'
import readline from 'node:readline'

const sessions = new Set()
const pendingClientRequests = new Map()

let nextClientRequestId = 1
let protocolVersion = 1
let platformCapabilities = null

const input = readline.createInterface({
  input: process.stdin,
  crlfDelay: Infinity,
})

input.on('line', (line) => {
  const trimmed = line.trim()
  if (!trimmed) {
    return
  }

  let message
  try {
    message = JSON.parse(trimmed)
  } catch (error) {
    console.error(`temporal-agent: failed to parse JSON-RPC line: ${error}`)
    return
  }

  if (typeof message.method === 'string') {
    void handleRequest(message)
    return
  }

  if (Object.prototype.hasOwnProperty.call(message, 'id')) {
    const key = requestKey(message.id)
    const pending = pendingClientRequests.get(key)
    if (!pending) {
      return
    }
    pendingClientRequests.delete(key)
    if (message.error) {
      pending.reject(new Error(message.error.message ?? 'client request failed'))
      return
    }
    pending.resolve(message.result)
  }
})

input.on('close', () => process.exit(0))

async function handleRequest(message) {
  switch (message.method) {
    case 'initialize':
      protocolVersion = message.params?.protocolVersion ?? 1
      platformCapabilities =
        message.params?.serverCapabilities?.platform ??
        message.params?._meta?.serverCapabilities?.platform ??
        null
      respond(message.id, {
        protocolVersion,
        agentCapabilities: {
          loadSession: false,
          promptCapabilities: {
            image: false,
            audio: false,
            embeddedContext: false,
          },
          mcpCapabilities: {
            http: false,
            sse: false,
          },
          sessionCapabilities: {},
        },
        agentInfo: {
          name: 'temporal-agent',
          title: 'Temporal Agent',
          version: '0.0.1',
        },
        authMethods: [],
      })
      return

    case 'session/new': {
      const sessionId = randomUUID()
      sessions.add(sessionId)
      respond(message.id, { sessionId })
      return
    }

    case 'session/prompt':
      await handlePrompt(message)
      return

    default:
      respondError(message.id, -32601, `method not found: ${message.method}`)
  }
}

async function handlePrompt(message) {
  const sessionId = message.params?.sessionId
  if (!sessions.has(sessionId)) {
    respondError(message.id, -32001, 'session_not_found', { sessionId })
    return
  }

  const text = extractPromptText(message.params?.prompt)
  const normalized = text.toLowerCase()

  try {
    if (normalized === 'wait 5s') {
      await invokeTemporalPrimitive(
        'session/wait',
        { sessionId, ms: 5000, durationMs: 5000 },
        sessionId,
        'waited 5 seconds',
      )
      respond(message.id, { stopReason: 'end_turn' })
      return
    }

    if (normalized === 'schedule hello in 10s') {
      await invokeTemporalPrimitive(
        'session/schedule',
        {
          sessionId,
          delayMs: 10000,
          ms: 10000,
          prompt: [{ type: 'text', text: 'hello' }],
        },
        sessionId,
        'scheduled "hello" in 10 seconds',
      )
      respond(message.id, { stopReason: 'end_turn' })
      return
    }

    if (normalized === 'wait for event') {
      await invokeTemporalPrimitive(
        'session/wait_for',
        {
          sessionId,
          filter: { kind: 'event', name: 'demo.temporal' },
        },
        sessionId,
        'wait_for resolved for demo.temporal',
      )
      respond(message.id, { stopReason: 'end_turn' })
      return
    }

    notifyText(sessionId, text || 'echo')
    respond(message.id, { stopReason: 'end_turn' })
  } catch (error) {
    respondError(message.id, -32000, error instanceof Error ? error.message : String(error))
  }
}

async function invokeTemporalPrimitive(method, params, sessionId, successText) {
  if (!supportsTemporalMethod(method)) {
    notifyText(sessionId, `${method} not advertised in serverCapabilities.platform; echo fallback`)
    return
  }

  await callClient(method, params)
  notifyText(sessionId, successText)
}

function supportsTemporalMethod(method) {
  const temporal =
    platformCapabilities?.temporal?.temporal ?? platformCapabilities?.temporal ?? platformCapabilities ?? {}
  const methods = new Set()

  if (Array.isArray(temporal.methods)) {
    for (const item of temporal.methods) {
      if (typeof item === 'string') {
        methods.add(item)
      }
    }
  }

  if (Array.isArray(temporal.availableMethods)) {
    for (const item of temporal.availableMethods) {
      if (typeof item === 'string') {
        methods.add(item)
      }
    }
  }

  if (temporal.wait === true || temporal.wait?.available === true || temporal.wait?.enabled === true) {
    methods.add('session/wait')
  }
  if (
    temporal.schedule === true ||
    temporal.schedule?.available === true ||
    temporal.schedule?.enabled === true
  ) {
    methods.add('session/schedule')
  }
  if (
    temporal.wait_for === true ||
    temporal.wait_for?.available === true ||
    temporal.wait_for?.enabled === true ||
    temporal.waitFor === true ||
    temporal.waitFor?.available === true ||
    temporal.waitFor?.enabled === true
  ) {
    methods.add('session/wait_for')
  }

  return methods.has(method)
}

function extractPromptText(prompt) {
  if (!Array.isArray(prompt)) {
    return ''
  }

  return prompt
    .map((block) => (block?.type === 'text' && typeof block.text === 'string' ? block.text : ''))
    .filter(Boolean)
    .join('\n')
    .trim()
}

function notifyText(sessionId, text) {
  send({
    jsonrpc: '2.0',
    method: 'session/update',
    params: {
      sessionId,
      update: {
        sessionUpdate: 'agent_message_chunk',
        content: {
          type: 'text',
          text,
        },
      },
    },
  })
}

function callClient(method, params) {
  const id = nextClientRequestId++
  send({
    jsonrpc: '2.0',
    id,
    method,
    params,
  })
  return new Promise((resolve, reject) => {
    pendingClientRequests.set(requestKey(id), { resolve, reject })
  })
}

function respond(id, result) {
  send({ jsonrpc: '2.0', id, result })
}

function respondError(id, code, message, data) {
  send({
    jsonrpc: '2.0',
    id,
    error: {
      code,
      message,
      ...(data === undefined ? {} : { data }),
    },
  })
}

function send(message) {
  process.stdout.write(`${JSON.stringify(message)}\n`)
}

function requestKey(id) {
  return `${typeof id}:${String(id)}`
}
