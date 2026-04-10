/**
 * @fireline/client
 *
 * Programmatic client API for Fireline.
 *
 * Provides:
 * - `FirelineClient` — the main entry point that holds a transport,
 *   a connection to the durable stream (via `@fireline/state`'s
 *   `createFirelineDB`), and a session manager
 * - `Session` — wraps an ACP `ClientSideConnection` and exposes
 *   `prompt()`, `onUpdate()`, `cancel()`, `close()`, etc.
 * - Transport constructors: `WebSocketTransport.connect(url)`,
 *   `StdioTransport.spawn(cmd, args)`, `InMemoryTransport.connect(...)`
 *
 * The transport is a parameter, not a URL — same pattern agent-os
 * uses with `AcpClient(process, stdoutLines)`. Any duplex byte
 * channel can become a transport.
 */

// TODO: implement FirelineClient, Session, transports
//
// Target shape:
//
// ```ts
// import { FirelineClient, WebSocketTransport } from '@fireline/client'
//
// const client = new FirelineClient({
//   streamUrl: 'http://localhost:4437/streams/fireline-main',
//   transport: WebSocketTransport.connect('ws://localhost:4438/acp'),
// })
//
// await client.connect()
// const session = await client.sessions.create({ cwd: '/' })
// session.onUpdate((update) => { /* ... */ })
// await session.prompt('Write a hello world script')
// ```

export const firelineClientPlaceholder = 'TODO: implement FirelineClient'
