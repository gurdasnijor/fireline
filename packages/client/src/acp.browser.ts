import type { AnyMessage } from '@agentclientprotocol/sdk/dist/jsonrpc.js'
import type { Stream } from '@agentclientprotocol/sdk'

import {
  createOpenAcpConnection,
  type AcpConnectOptions,
  type AcpInitializeOptions,
  type AcpSocketHandle,
  type OpenAcpConnection,
} from './acp-core.js'

const DEBUG_ACP = readBrowserDebugFlag()

export type { AcpConnectOptions, AcpInitializeOptions, OpenAcpConnection }

export async function connectAcp(options: AcpConnectOptions): Promise<OpenAcpConnection> {
  if (options.headers && Object.keys(options.headers).length > 0) {
    throw new Error('browser ACP transport does not support custom WebSocket headers')
  }

  const socket = await openWebSocket(options.url)
  return createOpenAcpConnection(createSocketHandle(socket), { debug: DEBUG_ACP })
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
    writable: new WritableStream<AnyMessage>({
      write(message) {
        if (DEBUG_ACP) {
          console.error('[fireline/acp] ->', JSON.stringify(message))
        }
        socket.send(JSON.stringify(message))
      },
      close() {
        socket.close()
      },
      abort() {
        socket.close()
      },
    }),
    readable: new ReadableStream<AnyMessage>({
      start(controller) {
        const onMessage = (event: MessageEvent<Blob | ArrayBuffer | string>) => {
          void toText(event.data)
            .then((text) => {
              const message = JSON.parse(text) as AnyMessage
              if (DEBUG_ACP) {
                console.error('[fireline/acp] <-', JSON.stringify(message))
              }
              controller.enqueue(message)
            })
            .catch((error) => {
              cleanup()
              controller.error(error)
            })
        }

        const onClose = () => {
          cleanup()
          controller.close()
        }

        const onError = () => {
          cleanup()
          controller.error(new Error('WebSocket error'))
        }

        const cleanup = () => {
          socket.removeEventListener('message', onMessage)
          socket.removeEventListener('close', onClose)
          socket.removeEventListener('error', onError)
        }

        socket.addEventListener('message', onMessage)
        socket.addEventListener('close', onClose, { once: true })
        socket.addEventListener('error', onError, { once: true })
      },
      cancel() {
        socket.close()
      },
    }),
  }
}

async function openWebSocket(url: string): Promise<WebSocket> {
  const socket = new WebSocket(url)

  await new Promise<void>((resolve, reject) => {
    const onOpen = () => {
      cleanup()
      resolve()
    }

    const onError = () => {
      cleanup()
      reject(new Error('WebSocket failed to open'))
    }

    const cleanup = () => {
      socket.removeEventListener('open', onOpen)
      socket.removeEventListener('error', onError)
    }

    socket.addEventListener('open', onOpen, { once: true })
    socket.addEventListener('error', onError, { once: true })
  })

  return socket
}

async function waitForWebSocketClose(socket: WebSocket): Promise<void> {
  if (socket.readyState === WebSocket.CLOSED) {
    return
  }

  await new Promise<void>((resolve) => {
    const onClose = () => {
      socket.removeEventListener('close', onClose)
      socket.removeEventListener('error', onClose)
      resolve()
    }

    socket.addEventListener('close', onClose, { once: true })
    socket.addEventListener('error', onClose, { once: true })
  })
}

async function toText(data: Blob | ArrayBuffer | string): Promise<string> {
  if (typeof data === 'string') {
    return data
  }
  if (data instanceof Blob) {
    return await data.text()
  }
  return new TextDecoder().decode(data)
}

function readBrowserDebugFlag(): boolean {
  try {
    return globalThis.localStorage?.getItem('FIRELINE_DEBUG_ACP') === '1'
  } catch {
    return false
  }
}
