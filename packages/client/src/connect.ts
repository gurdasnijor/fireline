import {
  ClientSideConnection,
  PROTOCOL_VERSION,
  type Client,
  type Stream,
} from '@agentclientprotocol/sdk'
import WebSocket, { type RawData } from 'ws'

import type { Endpoint } from './types.js'

const DEFAULT_CLIENT_NAME = '@fireline/client'
const DEFAULT_CLIENT_VERSION = '0.0.1'

export type ConnectedAcp = ClientSideConnection & {
  close(): Promise<void>
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

export async function connectAcp(
  endpoint: Endpoint | string,
  clientName = DEFAULT_CLIENT_NAME,
): Promise<ConnectedAcp> {
  const { url, headers } =
    typeof endpoint === 'string' ? { url: endpoint, headers: undefined } : endpoint

  const socket = new WebSocket(url, headers ? { headers } : undefined)

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
    close: () => closeSocket(socket),
  })
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
