import { type Stream } from '@agentclientprotocol/sdk'
import type { AnyMessage } from '@agentclientprotocol/sdk/dist/jsonrpc.js'
import WebSocket, { type RawData } from 'ws'

import {
  createOpenAcpConnection,
  type AcpConnectOptions,
  type AcpInitializeOptions,
  type AcpSocketHandle,
  type OpenAcpConnection,
} from './acp-core.js'

type JsonRpcMessage = AnyMessage

const DEBUG_ACP = process.env.FIRELINE_DEBUG_ACP === '1'

export type { AcpConnectOptions, AcpInitializeOptions, OpenAcpConnection }

export async function connectAcp(options: AcpConnectOptions): Promise<OpenAcpConnection> {
  const websocket = await openWebSocket(options)
  return createOpenAcpConnection(createSocketHandle(websocket), { debug: DEBUG_ACP })
}

function createSocketHandle(socket: WebSocket): AcpSocketHandle {
  return {
    stream: createWebSocketStream(socket),
    isConnecting() {
      return socket.readyState === WebSocket.CONNECTING
    },
    isOpen() {
      return socket.readyState === WebSocket.OPEN
    },
    isClosing() {
      return socket.readyState === WebSocket.CLOSING
    },
    close() {
      socket.close()
    },
    waitForClose() {
      return waitForWebSocketClose(socket)
    },
  }
}

function createWebSocketStream(socket: WebSocket): Stream {
  return {
    writable: new WritableStream<JsonRpcMessage>({
      write(message) {
        if (DEBUG_ACP) {
          console.error('[fireline/acp] ->', JSON.stringify(message))
        }
        return new Promise<void>((resolve, reject) => {
          socket.send(JSON.stringify(message), (error?: Error) => {
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
    readable: new ReadableStream<JsonRpcMessage>({
      start(controller) {
        const onMessage = (data: RawData) => {
          try {
            const message = parseMessagePayload(data)
            if (DEBUG_ACP) {
              console.error('[fireline/acp] <-', JSON.stringify(message))
            }
            controller.enqueue(message)
          } catch (error) {
            cleanup()
            controller.error(error)
          }
        }

        const onClose = () => {
          cleanup()
          controller.close()
        }

        const onError = (error: Error) => {
          cleanup()
          controller.error(error)
        }

        const cleanup = () => {
          socket.off('message', onMessage)
          socket.off('close', onClose)
          socket.off('error', onError)
        }

        socket.on('message', onMessage)
        socket.on('close', onClose)
        socket.on('error', onError)
      },
      cancel() {
        socket.close()
      },
    }),
  }
}

async function openWebSocket(options: AcpConnectOptions): Promise<WebSocket> {
  const socket = new WebSocket(options.url, {
    headers: options.headers,
  })

  await new Promise<void>((resolve, reject) => {
    const cleanup = () => {
      socket.off('open', onOpen)
      socket.off('error', onError)
    }

    const onOpen = () => {
      cleanup()
      resolve()
    }

    const onError = (error: Error) => {
      cleanup()
      reject(error)
    }

    socket.on('open', onOpen)
    socket.on('error', onError)
  })

  return socket
}

async function waitForWebSocketClose(socket: WebSocket): Promise<void> {
  if (socket.readyState === WebSocket.CLOSED) {
    return
  }

  await new Promise<void>((resolve) => {
    const cleanup = () => {
      socket.off('close', onClose)
      socket.off('error', onError)
    }

    const onClose = () => {
      cleanup()
      resolve()
    }

    const onError = () => {
      cleanup()
      resolve()
    }

    socket.on('close', onClose)
    socket.on('error', onError)
  })
}

function parseMessagePayload(data: RawData): JsonRpcMessage {
  if (typeof data === 'string') {
    return JSON.parse(data) as JsonRpcMessage
  }
  if (data instanceof ArrayBuffer) {
    return JSON.parse(Buffer.from(data).toString('utf8')) as JsonRpcMessage
  }
  if (Array.isArray(data)) {
    return JSON.parse(Buffer.concat(data).toString('utf8')) as JsonRpcMessage
  }
  return JSON.parse(data.toString('utf8')) as JsonRpcMessage
}
