import {
  ClientSideConnection,
  PROTOCOL_VERSION,
  type Client,
  type ClientCapabilities,
  type InitializeRequest,
  type InitializeResponse,
  type RequestPermissionRequest,
  type RequestPermissionResponse,
  type SessionNotification,
  type Stream,
} from '@agentclientprotocol/sdk'
import type { AnyMessage } from '@agentclientprotocol/sdk/dist/jsonrpc.js'
import WebSocket, { type RawData } from 'ws'

const DEBUG_ACP = process.env.FIRELINE_DEBUG_ACP === '1'

export interface AcpConnectOptions {
  url: string
  headers?: Record<string, string>
}

export interface AcpInitializeOptions {
  meta?: Record<string, unknown>
  clientCapabilities?: ClientCapabilities
  clientInfo?: {
    name: string
    version: string
    title?: string
  }
}

export interface OpenAcpConnection {
  connection: ClientSideConnection
  initialize(options?: AcpInitializeOptions): Promise<InitializeResponse>
  updates(): AsyncIterable<SessionNotification>
  close(): Promise<void>
}

interface Deferred<T> {
  promise: Promise<T>
  resolve(value: T): void
  reject(reason?: unknown): void
}

type JsonRpcMessage = AnyMessage


export async function connectAcp(options: AcpConnectOptions): Promise<OpenAcpConnection> {
  const websocket = await openWebSocket(options)
  const updateQueue = new AsyncQueue<SessionNotification>()
  const stream = createWebSocketStream(websocket)
  const connection = new ClientSideConnection(
    () =>
      createClientHandler({
        updateQueue,
      }),
    stream,
  )

  connection.signal.addEventListener(
    'abort',
    () => {
      updateQueue.close()
    },
    { once: true },
  )

  return {
    connection,

    async initialize(init = {}) {
      const request: InitializeRequest = {
        protocolVersion: PROTOCOL_VERSION,
        _meta: init.meta ?? null,
        clientCapabilities: init.clientCapabilities,
        clientInfo: {
          name: init.clientInfo?.name ?? '@fireline/client',
          version: init.clientInfo?.version ?? '0.0.1',
          title: init.clientInfo?.title,
        },
      }
      return connection.initialize(request)
    },

    updates() {
      return updateQueue.iterate()
    },

    async close() {
      updateQueue.close()
      if (websocket.readyState === WebSocket.OPEN || websocket.readyState === WebSocket.CONNECTING) {
        await closeWebSocket(websocket)
        return
      }
      if (websocket.readyState === WebSocket.CLOSING) {
        await waitForWebSocketClose(websocket)
      }
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

function createClientHandler(options: { updateQueue: AsyncQueue<SessionNotification> }): Client {
  return {
    async requestPermission(_params: RequestPermissionRequest): Promise<RequestPermissionResponse> {
      return {
        outcome: {
          outcome: 'cancelled',
        },
      }
    },

    async sessionUpdate(params: SessionNotification): Promise<void> {
      if (DEBUG_ACP) {
        console.error('[fireline/acp] sessionUpdate', JSON.stringify(params))
      }
      options.updateQueue.push(params)
    },

    async writeTextFile(): Promise<never> {
      throw new Error('client file system write is not implemented')
    },

    async readTextFile(): Promise<never> {
      throw new Error('client file system read is not implemented')
    },

    async createTerminal(): Promise<never> {
      throw new Error('client terminal create is not implemented')
    },

    async terminalOutput(): Promise<never> {
      throw new Error('client terminal output is not implemented')
    },

    async releaseTerminal(): Promise<never> {
      throw new Error('client terminal release is not implemented')
    },

    async waitForTerminalExit(): Promise<never> {
      throw new Error('client terminal wait is not implemented')
    },

    async killTerminal(): Promise<never> {
      throw new Error('client terminal kill is not implemented')
    },

    async extMethod(method: string): Promise<Record<string, unknown>> {
      throw new Error(`client extension method '${method}' is not implemented`)
    },

    async extNotification(): Promise<void> {
      // Ignore unknown extension notifications in the primitive client.
    },
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

async function closeWebSocket(socket: WebSocket): Promise<void> {
  const closePromise = waitForWebSocketClose(socket)
  socket.close()
  await Promise.race([closePromise, sleep(1_000)])
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

class AsyncQueue<T> {
  private values: T[] = []
  private waiters: Deferred<IteratorResult<T>>[] = []
  private closed = false

  push(value: T): void {
    if (this.closed) {
      return
    }

    const waiter = this.waiters.shift()
    if (waiter) {
      waiter.resolve({ value, done: false })
      return
    }

    this.values.push(value)
  }

  close(): void {
    if (this.closed) {
      return
    }
    this.closed = true
    while (this.waiters.length > 0) {
      this.waiters.shift()?.resolve({ value: undefined, done: true })
    }
  }

  iterate(): AsyncIterable<T> {
    const queue = this
    return {
      [Symbol.asyncIterator]() {
        return {
          async next(): Promise<IteratorResult<T>> {
            if (queue.values.length > 0) {
              const value = queue.values.shift() as T
              return { value, done: false }
            }

            if (queue.closed) {
              return { value: undefined, done: true }
            }

            return queue.wait()
          },
        }
      },
    }
  }

  private wait(): Promise<IteratorResult<T>> {
    const deferred = createDeferred<IteratorResult<T>>()
    this.waiters.push(deferred)
    return deferred.promise
  }
}

function createDeferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void

  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })

  return { promise, resolve, reject }
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, ms)
  })
}
