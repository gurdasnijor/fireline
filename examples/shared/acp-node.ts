// This is NOT a Fireline library. It's a documented recipe for connecting to
// an ACP endpoint from Node. For React/browser, use 'use-acp' instead.
// See: https://github.com/marimo-team/use-acp
import {
  ClientSideConnection,
  PROTOCOL_VERSION,
  type Client,
  type Stream,
} from '@agentclientprotocol/sdk'
import WebSocket, { type RawData } from 'ws'

const unsupported = (name: string): never => {
  throw new Error(`ACP SDK requires stub client method '${name}'. For React/browser, use use-acp instead.`)
}
const client = {
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

export async function openNodeAcpConnection(url: string, clientName: string) {
  const socket = new WebSocket(url)
  await new Promise<void>((resolve, reject) => { socket.once('open', () => resolve()); socket.once('error', reject) })
  const stream: Stream = {
    readable: new ReadableStream({ start(controller) { socket.on('message', (data: RawData) => controller.enqueue(JSON.parse(Buffer.isBuffer(data) ? data.toString('utf8') : String(data)))); socket.once('close', () => controller.close()); socket.once('error', (error: Error) => controller.error(error)) } }),
    writable: new WritableStream({ write(message) { socket.send(JSON.stringify(message)) }, close() { socket.close() }, abort() { socket.close() } }),
  }
  const connection = new ClientSideConnection(() => client, stream)
  await connection.initialize({ protocolVersion: PROTOCOL_VERSION, clientInfo: { name: clientName, version: '0.0.1' }, clientCapabilities: { fs: { readTextFile: false } } })
  return { connection, close: () => new Promise<void>((resolve) => { socket.once('close', () => resolve()); socket.close() }) }
}
