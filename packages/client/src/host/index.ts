/**
 * Public Host primitive types for session lifecycle management, as specified in
 * `docs/proposals/client-primitives.md` and anchored to
 * `docs/explorations/managed-agents-mapping.md`.
 */
import type { SessionSpec } from '../core/session-spec.js'
import type {
  SessionHandle,
  SessionInput,
  SessionOutput,
  SessionStatus,
  WakeOutcome,
} from '../core/session.js'

export type {
  SessionHandle,
  SessionInput,
  SessionOutput,
  SessionStatus,
  WakeOutcome,
} from '../core/session.js'

export interface Host {
  createSession(spec: SessionSpec): Promise<SessionHandle>
  wake(handle: SessionHandle): Promise<WakeOutcome>
  status(handle: SessionHandle): Promise<SessionStatus>
  stopSession(handle: SessionHandle): Promise<void>
  sendInput?(handle: SessionHandle, input: SessionInput): AsyncIterable<SessionOutput>
}
