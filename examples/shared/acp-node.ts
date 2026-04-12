// User code
import { createRequire } from 'node:module'

type TextPrompt = { type: 'text'; text: string }

export interface NodeAcpConnection {
  connection: {
    initialize(opts: object): Promise<unknown>
    newSession(opts: { cwd: string; mcpServers: unknown[] }): Promise<{ sessionId: string }>
    prompt(opts: { sessionId: string; prompt: TextPrompt[] }): Promise<unknown>
    loadSession(opts: { sessionId: string; cwd: string; mcpServers: unknown[] }): Promise<unknown>
  }
  close(): Promise<void>
}

export async function openNodeAcpConnection(
  callerUrl: string,
  url: string,
  clientName: string,
): Promise<NodeAcpConnection> {
  const requireFromCaller = createRequire(callerUrl)
  const sdk = (await import(requireFromCaller.resolve('@agentclientprotocol/sdk'))) as {
    ClientSideConnection: new (client: () => object, stream: object) => NodeAcpConnection['connection']
    PROTOCOL_VERSION: number
  }
  const wsModule = (await import(requireFromCaller.resolve('ws'))) as {
    default: new (url: string) => {
      readyState: number
      once(event: string, cb: (...args: unknown[]) => void): void
      on(event: string, cb: (...args: unknown[]) => void): void
      send(payload: string, cb?: (error?: Error) => void): void
      close(): void
    }
  }
  const socket = new wsModule.default(url)

  await new Promise<void>((resolve, reject) => {
    socket.once('open', () => resolve())
    socket.once('error', (error) => reject(error))
  })

  const connection = new sdk.ClientSideConnection(() => createClient(), createStream(socket))
  await connection.initialize({
    protocolVersion: sdk.PROTOCOL_VERSION,
    clientInfo: { name: clientName, version: '0.0.1' },
    clientCapabilities: { fs: { readTextFile: false } },
  })

  return {
    connection,
    async close() {
      socket.close()
      await new Promise<void>((resolve) => socket.once('close', () => resolve()))
    },
  }
}

function createClient() {
  return {
    async requestPermission() {
      return { outcome: { outcome: 'cancelled' } }
    },
    async sessionUpdate() {},
    async writeTextFile(): Promise<never> { throw new Error('not implemented') },
    async readTextFile(): Promise<never> { throw new Error('not implemented') },
    async createTerminal(): Promise<never> { throw new Error('not implemented') },
    async terminalOutput(): Promise<never> { throw new Error('not implemented') },
    async releaseTerminal(): Promise<never> { throw new Error('not implemented') },
    async waitForTerminalExit(): Promise<never> { throw new Error('not implemented') },
    async killTerminal(): Promise<never> { throw new Error('not implemented') },
    async extMethod(method: string): Promise<Record<string, unknown>> {
      throw new Error(`unsupported ext method: ${method}`)
    },
    async extNotification() {},
  }
}

function createStream(socket: {
  on(event: string, cb: (...args: unknown[]) => void): void
  once(event: string, cb: (...args: unknown[]) => void): void
  send(payload: string, cb?: (error?: Error) => void): void
  close(): void
}) {
  return {
    writable: new WritableStream({
      write(message) {
        return new Promise<void>((resolve, reject) => {
          socket.send(JSON.stringify(message), (error) => (error ? reject(error) : resolve()))
        })
      },
      close() {
        socket.close()
      },
      abort() {
        socket.close()
      },
    }),
    readable: new ReadableStream({
      start(controller) {
        socket.on('message', (data) => controller.enqueue(JSON.parse(toText(data))))
        socket.once('close', () => controller.close())
        socket.once('error', (error) => controller.error(error))
      },
      cancel() {
        socket.close()
      },
    }),
  }
}

function toText(data: unknown): string {
  if (typeof data === 'string') {
    return data
  }
  if (data instanceof ArrayBuffer) {
    return Buffer.from(data).toString('utf8')
  }
  if (Array.isArray(data)) {
    return Buffer.concat(data as Uint8Array[]).toString('utf8')
  }
  return Buffer.from(data as Uint8Array).toString('utf8')
}
