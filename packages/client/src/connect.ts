import {
  ClientSideConnection,
  PROTOCOL_VERSION,
  type Client,
  type Stream,
} from '@agentclientprotocol/sdk'
import WebSocket, { type RawData } from 'ws'

import type { Endpoint, WebSocketTransport } from './types.js'

const DEFAULT_CLIENT_NAME = '@fireline/client'
const DEFAULT_CLIENT_VERSION = '0.0.1'

export type ConnectedAcp = ClientSideConnection & {
  close(): Promise<void>
}

interface SpawnedStdioTransport {
  readonly command: string
  readonly args: readonly string[]
  readonly cwd?: string
  readonly env?: Readonly<Record<string, string>>
}

const unsupported = (name: string): never => {
  throw new Error(
    `ACP SDK requires stub client method '${name}'. Build on @agentclientprotocol/sdk directly for custom client behavior.`,
  )
}

const defaultClient = {
  requestPermission: async () => ({ outcome: { outcome: 'cancelled' } }),
  sessionUpdate: async () => {},
  writeTextFile: async () => unsupported('writeTextFile'),
  readTextFile: async () => unsupported('readTextFile'),
  createTerminal: async () => unsupported('createTerminal'),
  terminalOutput: async () => unsupported('terminalOutput'),
  releaseTerminal: async () => unsupported('releaseTerminal'),
  waitForTerminalExit: async () => unsupported('waitForTerminalExit'),
  killTerminal: async () => unsupported('killTerminal'),
  extMethod: async (method: string) => unsupported(`extMethod:${method}`),
  extNotification: async () => {},
} satisfies Client

/**
 * Low-level websocket ACP connector retained for the migration window.
 *
 * @deprecated Prefer `compose(...).connect_to({ kind: 'hosted' | 'websocket' | 'stream' | 'stdio', ... })`.
 */
export async function connectAcp(
  endpoint: Endpoint | string,
  clientName = DEFAULT_CLIENT_NAME,
): Promise<ConnectedAcp> {
  const transport =
    typeof endpoint === 'string'
      ? { kind: 'websocket' as const, url: endpoint }
      : {
          kind: 'websocket' as const,
          url: endpoint.url,
          headers: endpoint.headers,
        }
  return connectWebSocket(transport, clientName)
}

export async function connectWebSocket(
  transport: WebSocketTransport,
  clientName = transport.clientName ?? DEFAULT_CLIENT_NAME,
): Promise<ConnectedAcp> {
  const socket = new WebSocket(
    transport.url,
    transport.headers ? { headers: transport.headers } : undefined,
  )

  await new Promise<void>((resolve, reject) => {
    const handleOpen = () => {
      socket.off('error', handleError)
      resolve()
    }
    const handleError = (error: Error) => {
      socket.off('open', handleOpen)
      reject(error)
    }

    socket.once('open', handleOpen)
    socket.once('error', handleError)
  })

  const stream: Stream = {
    readable: new ReadableStream({
      start(controller) {
        const handleMessage = (data: RawData) => {
          controller.enqueue(JSON.parse(decodeRawData(data)))
        }
        const handleClose = () => {
          cleanup()
          controller.close()
        }
        const handleError = (error: Error) => {
          cleanup()
          controller.error(error)
        }
        const cleanup = () => {
          socket.off('message', handleMessage)
          socket.off('close', handleClose)
          socket.off('error', handleError)
        }

        socket.on('message', handleMessage)
        socket.on('close', handleClose)
        socket.on('error', handleError)
      },
    }),
    writable: new WritableStream({
      write(message) {
        return new Promise<void>((resolve, reject) => {
          socket.send(JSON.stringify(message), (error) => {
            if (error) {
              reject(error)
              return
            }
            resolve()
          })
        })
      },
      close() {
        socket.close()
      },
      abort() {
        socket.close()
      },
    }),
  }

  return connectStream(stream, clientName, () => closeSocket(socket))
}

export async function connectStream(
  stream: Stream,
  clientName = DEFAULT_CLIENT_NAME,
  close = () => closeReadableWritable(stream),
): Promise<ConnectedAcp> {
  const connection = new ClientSideConnection(() => defaultClient, stream)
  await connection.initialize({
    protocolVersion: PROTOCOL_VERSION,
    clientInfo: {
      name: clientName,
      version: DEFAULT_CLIENT_VERSION,
    },
    clientCapabilities: {
      fs: { readTextFile: false },
    },
  })

  return Object.assign(connection, {
    close,
  })
}

export async function connectSpawnedStdio(
  transport: SpawnedStdioTransport,
  clientName = DEFAULT_CLIENT_NAME,
): Promise<ConnectedAcp> {
  const { spawn } = await import('node:child_process')
  const child = spawn(transport.command, [...transport.args], {
    cwd: transport.cwd,
    env: {
      ...process.env,
      ...transport.env,
    },
    stdio: ['pipe', 'pipe', 'inherit'],
  })

  if (!child.stdin || !child.stdout) {
    throw new Error('spawned stdio transport missing stdin/stdout pipes')
  }

  const stream = jsonLineStream(child.stdout, child.stdin)
  const connection = await connectStream(stream, clientName, async () => {
    await closeReadableWritable(stream)
    if (!child.killed && child.exitCode === null && child.signalCode === null) {
      child.kill('SIGTERM')
    }
    await new Promise<void>((resolve) => {
      child.once('exit', () => resolve())
      setTimeout(resolve, 5_000)
    })
  })

  return connection
}

async function closeSocket(socket: WebSocket): Promise<void> {
  if (socket.readyState === WebSocket.CLOSED) {
    return
  }

  await new Promise<void>((resolve) => {
    socket.once('close', () => resolve())
    socket.close()
  })
}

async function closeReadableWritable(stream: Stream): Promise<void> {
  try {
    const writer = stream.writable.getWriter()
    try {
      await writer.close()
    } finally {
      writer.releaseLock()
    }
  } catch {
    // Ignore close races on caller-owned transports.
  }

  try {
    const reader = stream.readable.getReader()
    try {
      await reader.cancel()
    } finally {
      reader.releaseLock()
    }
  } catch {
    // Ignore cancellation races on caller-owned transports.
  }
}

function jsonLineStream(
  stdout: NodeJS.ReadableStream,
  stdin: NodeJS.WritableStream,
): Stream {
  let buffer = ''

  return {
    readable: new ReadableStream({
      start(controller) {
        const handleData = (chunk: Buffer | string) => {
          buffer += typeof chunk === 'string' ? chunk : chunk.toString('utf8')
          while (true) {
            const newlineIndex = buffer.indexOf('\n')
            if (newlineIndex === -1) {
              break
            }
            const line = buffer.slice(0, newlineIndex).trim()
            buffer = buffer.slice(newlineIndex + 1)
            if (!line) {
              continue
            }
            controller.enqueue(JSON.parse(line))
          }
        }
        const handleEnd = () => {
          cleanup()
          controller.close()
        }
        const handleError = (error: Error) => {
          cleanup()
          controller.error(error)
        }
        const cleanup = () => {
          stdout.off('data', handleData)
          stdout.off('end', handleEnd)
          stdout.off('close', handleEnd)
          stdout.off('error', handleError)
        }

        stdout.on('data', handleData)
        stdout.on('end', handleEnd)
        stdout.on('close', handleEnd)
        stdout.on('error', handleError)
      },
    }),
    writable: new WritableStream({
      write(message) {
        return new Promise<void>((resolve, reject) => {
          const encoded = `${JSON.stringify(message)}\n`
          stdin.write(encoded, (error) => {
            if (error) {
              reject(error)
              return
            }
            resolve()
          })
        })
      },
      close() {
        return new Promise<void>((resolve, reject) => {
          stdin.end((error?: Error | null) => {
            if (error) {
              reject(error)
              return
            }
            resolve()
          })
        })
      },
      abort() {
        const destroyable = stdin as NodeJS.WritableStream & {
          destroy?: (error?: Error) => void
        }
        destroyable.destroy?.()
      },
    }),
  }
}

function decodeRawData(data: RawData): string {
  if (typeof data === 'string') {
    return data
  }

  if (Buffer.isBuffer(data)) {
    return data.toString('utf8')
  }

  if (Array.isArray(data)) {
    return Buffer.concat(data).toString('utf8')
  }

  if (data instanceof ArrayBuffer) {
    return Buffer.from(data).toString('utf8')
  }

  return Buffer.concat(data).toString('utf8')
}
