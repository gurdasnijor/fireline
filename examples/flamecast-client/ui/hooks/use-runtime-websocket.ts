import { useCallback, useMemo } from "react";
import type {
  WsChannelControlMessage,
  WsChannelServerMessage,
} from "../ws-types.js";

export type ConnectionState = "disconnected" | "connecting" | "connected" | "reconnecting";

export type ChannelMessageHandler = (message: WsChannelServerMessage) => void;

export interface RuntimeWebSocketHandle {
  connectionState: ConnectionState;
  subscribe(
    channel: string,
    handler: ChannelMessageHandler,
    opts?: { getSince?: () => number },
  ): () => void;
  send(message: WsChannelControlMessage): void;
}

export function useRuntimeWebSocket(websocketUrl?: string): RuntimeWebSocketHandle {
  const connectionState: ConnectionState = websocketUrl ? "connected" : "disconnected";

  const subscribe = useCallback(
    (
      _channel: string,
      _handler: ChannelMessageHandler,
      _opts?: { getSince?: () => number },
    ) => {
      return () => {};
    },
    [],
  );

  const send = useCallback((_message: WsChannelControlMessage) => {
    // TODO(flamecast-client): terminal multiplexing was Flamecast-specific.
    // Interactive prompts now go through ACP directly in useSessionState.
  }, []);

  return useMemo(() => ({ connectionState, subscribe, send }), [connectionState, send, subscribe]);
}
