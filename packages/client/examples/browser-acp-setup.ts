import {
  ClientSideConnection,
  PROTOCOL_VERSION,
  type Client,
  type RequestPermissionRequest,
  type RequestPermissionResponse,
  type SessionNotification,
  type Stream,
} from '@agentclientprotocol/sdk'

/**
 * Canonical browser ACP setup recipe.
 *
 * This is deliberately an example, not part of the Fireline client API:
 * - Fireline gives you `handle.acp.url`
 * - the ACP SDK owns the transport + protocol client
 * - your app owns permission UX and session-update handling
 *
 * Copy this file into your app and customize the marked sections.
 */

export type PermissionResolver = (value: RequestPermissionResponse) => void

export interface DefaultClientHandlerOptions {
  /**
   * App-specific:
   * surface the permission request in your own UI and call `resolve(...)`
   * once the user makes a choice.
   */
  readonly onPermission?: (
    request: RequestPermissionRequest,
    resolve: PermissionResolver,
  ) => void

  /**
   * App-specific:
   * append notifications to your own event log, state store, or UI.
   */
  readonly onSessionUpdate?: (
    notification: SessionNotification,
  ) => void | Promise<void>
}

export interface BrowserAcpSetupOptions extends DefaultClientHandlerOptions {
  /**
   * Optional full override for the ACP initialize handshake.
   * Most apps can start with `defaultInitializeOptions` unchanged.
   */
  readonly initialize?: Parameters<ClientSideConnection['initialize']>[0]
}

export interface BrowserAcpConnection {
  readonly websocket: WebSocket
  readonly connection: ClientSideConnection
}

/**
 * Generic ACP browser setup:
 * 1. build a WebSocket from `handle.acp.url`
 * 2. adapt it into an ACP `Stream`
 * 3. create `ClientSideConnection`
 * 4. run `initialize()` with sane defaults
 *
 * App-specific parts are limited to:
 * - `onPermission`
 * - `onSessionUpdate`
 */
export async function openBrowserAcpConnection(
  handle: { readonly acp: { readonly url: string } },
  options: BrowserAcpSetupOptions = {},
): Promise<BrowserAcpConnection> {
  const websocket = new WebSocket(handle.acp.url)
  await waitForSocketOpen(websocket)

  const connection = new ClientSideConnection(
    () =>
      createDefaultClientHandler({
        onPermission: options.onPermission,
        onSessionUpdate: options.onSessionUpdate,
      }),
    createWebSocketStream(websocket),
  )

  await connection.initialize(options.initialize ?? defaultInitializeOptions)

  return { websocket, connection }
}

/**
 * Generic default ACP client handler for browser apps that only need:
 * - permission prompts
 * - session-update notifications
 *
 * The unsupported ACP client methods are intentionally stubbed. If your app
 * needs filesystem or terminal capabilities, replace those stubs with real
 * implementations.
 */
export function createDefaultClientHandler(
  options: DefaultClientHandlerOptions = {},
): Client {
  return {
    async requestPermission(
      request: RequestPermissionRequest,
    ): Promise<RequestPermissionResponse> {
      if (!options.onPermission) {
        return {
          outcome: {
            outcome: 'cancelled',
          },
        }
      }

      return await new Promise((resolve) => {
        options.onPermission?.(request, resolve)
      })
    },

    async sessionUpdate(notification: SessionNotification): Promise<void> {
      await options.onSessionUpdate?.(notification)
    },

    async writeTextFile(): Promise<never> {
      throw new Error('Browser example does not implement writeTextFile')
    },
    async readTextFile(): Promise<never> {
      throw new Error('Browser example does not implement readTextFile')
    },
    async createTerminal(): Promise<never> {
      throw new Error('Browser example does not implement createTerminal')
    },
    async terminalOutput(): Promise<never> {
      throw new Error('Browser example does not implement terminalOutput')
    },
    async releaseTerminal(): Promise<never> {
      throw new Error('Browser example does not implement releaseTerminal')
    },
    async waitForTerminalExit(): Promise<never> {
      throw new Error('Browser example does not implement waitForTerminalExit')
    },
    async killTerminal(): Promise<never> {
      throw new Error('Browser example does not implement killTerminal')
    },
    async extMethod(method: string): Promise<Record<string, unknown>> {
      throw new Error(`Browser example does not implement client ext method '${method}'`)
    },
    async extNotification(): Promise<void> {
      // Most thin browser clients can safely ignore unknown extension notifications.
    },
  }
}

/**
 * Generic transport adapter from browser `WebSocket` to ACP SDK `Stream`.
 */
export function createWebSocketStream(ws: WebSocket): Stream {
  return {
    readable: new ReadableStream({
      start(controller) {
        ws.addEventListener('message', (event) => {
          toText(event.data)
            .then((text) => controller.enqueue(JSON.parse(text)))
            .catch((error) => controller.error(error))
        })
        ws.addEventListener('close', () => controller.close(), { once: true })
        ws.addEventListener('error', () => controller.error(new Error('WebSocket error')), {
          once: true,
        })
      },
    }),
    writable: new WritableStream({
      write(message) {
        ws.send(JSON.stringify(message))
      },
      close() {
        ws.close()
      },
      abort() {
        ws.close()
      },
    }),
  }
}

/**
 * Generic helper for the browser WebSocket open handshake.
 */
export async function waitForSocketOpen(ws: WebSocket): Promise<void> {
  if (ws.readyState === WebSocket.OPEN) {
    return
  }

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
      ws.removeEventListener('open', onOpen)
      ws.removeEventListener('error', onError)
    }

    ws.addEventListener('open', onOpen, { once: true })
    ws.addEventListener('error', onError, { once: true })
  })
}

export async function waitForSocketClose(ws: WebSocket): Promise<void> {
  if (ws.readyState === WebSocket.CLOSED) {
    return
  }

  await new Promise<void>((resolve) => {
    ws.addEventListener('close', () => resolve(), { once: true })
  })
}

export async function toText(data: Blob | ArrayBuffer | string): Promise<string> {
  if (typeof data === 'string') {
    return data
  }
  if (data instanceof Blob) {
    return await data.text()
  }
  return new TextDecoder().decode(data)
}

export const defaultInitializeOptions = {
  protocolVersion: PROTOCOL_VERSION,
  clientCapabilities: { fs: { readTextFile: false } },
  clientInfo: {
    name: '@your-app/browser',
    version: '0.0.1',
    title: 'Your App Browser ACP Client',
  },
} satisfies Parameters<ClientSideConnection['initialize']>[0]

/**
 * Minimal usage sketch:
 *
 * ```ts
 * const { websocket, connection } = await openBrowserAcpConnection(handle, {
 *   onPermission(request, resolve) {
 *     showPermissionDialog(request, resolve)
 *   },
 *   onSessionUpdate(notification) {
 *     appendToEventLog(notification)
 *   },
 * })
 *
 * const session = await connection.newSession({ cwd: '/', mcpServers: [] })
 * // ...
 * websocket.close()
 * ```
 */
