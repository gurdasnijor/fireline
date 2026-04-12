import {
  ClientSideConnection,
  PROTOCOL_VERSION,
  type Client,
  type Stream,
} from '@agentclientprotocol/sdk'
import WebSocket from 'ws'

const unsupported = (name: string) => async (): Promise<never> => {
  throw new Error(`ACP client method '${name}' is not implemented in this example helper`)
}

const client: Client = {
  requestPermission: async () => ({ outcome: { outcome: 'cancelled' } }),
  sessionUpdate: async () => {},
  writeTextFile: unsupported('writeTextFile'),
  readTextFile: unsupported('readTextFile'),
  createTerminal: unsupported('createTerminal'),
  terminalOutput: unsupported('terminalOutput'),
  releaseTerminal: unsupported('releaseTerminal'),
  waitForTerminalExit: unsupported('waitForTerminalExit'),
  killTerminal: unsupported('killTerminal'),
  extMethod: unsupported('extMethod'),
  extNotification: async () => {},
}

export async function openNodeAcpConnection(url: string, clientName: string) {
  const socket = new WebSocket(url)
  await new Promise<void>((resolve, reject) => {
    socket.once('open', resolve)
    socket.once('error', reject)
  })

  const stream: Stream = {
    readable: new ReadableStream({
      start(controller) {
        socket.on('message', (data) => controller.enqueue(JSON.parse(data.toString())))
        socket.once('close', () => controller.close())
        socket.once('error', (error) => controller.error(error))
      },
    }),
    writable: new WritableStream({
      write(message) { socket.send(JSON.stringify(message)) },
      close() { socket.close() },
      abort() { socket.close() },
    }),
  }

  const connection = new ClientSideConnection(() => client, stream)
  await connection.initialize({
    protocolVersion: PROTOCOL_VERSION,
    clientInfo: { name: clientName, version: '0.0.1' },
    clientCapabilities: { fs: { readTextFile: false } },
  })
  return {
    connection,
    close: () =>
      new Promise<void>((resolve) => {
        socket.once('close', resolve)
        socket.close()
      }),
  }
}
