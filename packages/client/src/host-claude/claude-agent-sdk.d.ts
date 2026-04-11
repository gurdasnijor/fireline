/**
 * Ambient type stub for the Claude Agent SDK v2 preview surface.
 *
 * This exists so `@fireline/client/host-claude` can type-check in isolation
 * without forcing a runtime install of `@anthropic-ai/claude-agent-sdk`. The
 * shapes below are copied from the public V2 preview docs and from the
 * verified surface recorded in
 * `docs/explorations/claude-agent-sdk-v2-findings.md` (`§2 Programming model`).
 *
 * If / when the real package is installed as a peer dep, TypeScript's
 * module resolution will prefer the package's own `.d.ts` over this stub.
 *
 * Intentionally minimal: only the `unstable_v2_*` surface the
 * `createClaudeHost` satisfier consumes is declared. If additional V2
 * exports become load-bearing, extend this file rather than widening the
 * satisfier's dependency on runtime behaviour.
 */
declare module '@anthropic-ai/claude-agent-sdk' {
  /**
   * A single streamed message emitted by {@link SDKSession.stream}. The V2
   * docs guarantee every yielded message carries a `session_id`; beyond
   * that the concrete shape (`assistant` / `system` / `tool_use` / …) is
   * intentionally opaque here — the satisfier mirrors messages verbatim
   * into the durable stream rather than inspecting their type.
   */
  export interface SDKMessage {
    readonly type: string
    readonly session_id: string
    readonly [key: string]: unknown
  }

  /**
   * A single-turn user message accepted by {@link SDKSession.send}. The V2
   * docs accept either a plain string or a richer user-message object; we
   * only need the string form for the current satisfier.
   */
  export type SDKUserMessage = {
    readonly type: 'user'
    readonly content: string
    readonly [key: string]: unknown
  }

  /**
   * Result payload for the one-shot {@link unstable_v2_prompt} helper.
   * Opaque to this satisfier — included only so the export is callable.
   */
  export interface SDKResultMessage {
    readonly type: string
    readonly [key: string]: unknown
  }

  /**
   * Live, long-lived session handle returned by both
   * {@link unstable_v2_createSession} and {@link unstable_v2_resumeSession}.
   *
   * The satisfier holds one of these per `SessionHandle.id` in a
   * process-lifetime `Map`, and reconstructs it on miss via `acquire()`.
   */
  export interface SDKSession {
    readonly sessionId: string
    send(message: string | SDKUserMessage): Promise<void>
    stream(): AsyncGenerator<SDKMessage, void>
    close(): void
  }

  export interface SDKSessionOptions {
    readonly model: string
    // Additional options supported — see V2 preview docs.
    readonly [key: string]: unknown
  }

  export function unstable_v2_createSession(options: SDKSessionOptions): SDKSession

  export function unstable_v2_resumeSession(
    sessionId: string,
    options: SDKSessionOptions,
  ): SDKSession

  export function unstable_v2_prompt(
    prompt: string,
    options: SDKSessionOptions,
  ): Promise<SDKResultMessage>
}
