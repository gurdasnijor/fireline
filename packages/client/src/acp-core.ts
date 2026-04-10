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

export interface AcpSocketHandle {
  stream: Stream
  isConnecting(): boolean
  isOpen(): boolean
  isClosing(): boolean
  close(): void
  waitForClose(): Promise<void>
}

interface Deferred<T> {
  promise: Promise<T>
  resolve(value: T): void
  reject(reason?: unknown): void
}

export function createOpenAcpConnection(
  socket: AcpSocketHandle,
  options: { debug?: boolean } = {},
): OpenAcpConnection {
  const updateQueue = new AsyncQueue<SessionNotification>()
  const connection = new ClientSideConnection(
    () =>
      createClientHandler({
        debug: options.debug ?? false,
        updateQueue,
      }),
    socket.stream,
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
      if (socket.isOpen() || socket.isConnecting()) {
        socket.close()
        await Promise.race([socket.waitForClose(), sleep(1_000)])
        return
      }
      if (socket.isClosing()) {
        await socket.waitForClose()
      }
    },
  }
}

function createClientHandler(options: {
  debug: boolean
  updateQueue: AsyncQueue<SessionNotification>
}): Client {
  return {
    async requestPermission(_params: RequestPermissionRequest): Promise<RequestPermissionResponse> {
      return {
        outcome: {
          outcome: 'cancelled',
        },
      }
    },

    async sessionUpdate(params: SessionNotification): Promise<void> {
      if (options.debug) {
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
