import { useMemo } from "react";
import type { RuntimeWebSocketHandle } from "./use-runtime-websocket.js";

export type TerminalSession = {
  terminalId: string;
  command?: string;
  createdAt?: string;
  exitCode: number | null;
  output?: string;
};

export function useTerminal(_ws: RuntimeWebSocketHandle, _websocketUrl?: string) {
  return useMemo(
    () => ({
      terminals: [] as TerminalSession[],
      activeTerminal: null as TerminalSession | null,
      sendInput: (_terminalId: string, _data: string) => {},
      resize: (_terminalId: string, _cols: number, _rows: number) => {},
      onData: (_terminalId: string, _listener: (data: string) => void) => () => {},
      createTerminal: (_command?: string) => {},
      killTerminal: (_terminalId: string) => {},
    }),
    [],
  );
}
