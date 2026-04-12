// Third-party
import {
  ClientSideConnection,
  PROTOCOL_VERSION,
  type Client,
  type Stream,
} from '@agentclientprotocol/sdk'
import WebSocket, { type RawData } from 'ws'

export async function openNodeAcpConnection(url: string, clientName: string) {
  const socket = new WebSocket(url)
  await new Promise<void>((resolve, reject) => {
    socket.once('open', () => resolve())
    socket.once('error', (error: Error) => reject(error))
  })

  const connection = new ClientSideConnection(() => createClient(), createStream(socket))
  await connection.initialize({
    protocolVersion: PROTOCOL_VERSION,
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

function createClient(): Client {
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

function createStream(socket: WebSocket): Stream {
  return {
    writable: new WritableStream({
      write(message) {
        return new Promise<void>((resolve, reject) => {
          socket.send(JSON.stringify(message), (error?: Error) => (error ? reject(error) : resolve()))
        })
      },
      close() {
        socket.close()
      },
      abort() {
        socket.close()
      },
    }) as Stream['writable'],
    readable: new ReadableStream({
      start(controller) {
        socket.on('message', (data: RawData) => controller.enqueue(JSON.parse(toText(data))))
        socket.once('close', () => controller.close())
        socket.once('error', (error: Error) => controller.error(error))
      },
      cancel() {
        socket.close()
      },
    }) as Stream['readable'],
  }
}

function toText(data: RawData): string {
  if (typeof data === 'string') {
    return data
  }
  if (data instanceof ArrayBuffer) {
    return Buffer.from(data).toString('utf8')
  }
  if (Array.isArray(data)) {
    return Buffer.concat(data).toString('utf8')
  }
  return data.toString('utf8')
}
