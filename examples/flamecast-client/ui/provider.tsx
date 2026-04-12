import { createContext, useContext, useEffect, useMemo, useState } from "react";
import { db as openFirelineDb, type FirelineDB } from "@fireline/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createFlamecastClient } from "./fireline-client.js";
import type { FlamecastClient } from "./fireline-client.js";

const FlamecastContext = createContext<FlamecastClient | null>(null);
const FirelineDbContext = createContext<FirelineDB | null>(null);

export function FlamecastProvider({
  children,
  baseUrl,
}: {
  children: React.ReactNode;
  baseUrl: string;
}) {
  const queryClient = useMemo(() => new QueryClient(), []);
  const client = useMemo(() => createFlamecastClient({ baseUrl }), [baseUrl]);
  const [db, setDb] = useState<FirelineDB | null>(null);
  const [ready, setReady] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setReady(false);
    setDb(null);

    void fetchFirelineConfig(baseUrl).then(async (config) => {
      if (cancelled) return;
      const nextDb = await openFirelineDb({ stateStreamUrl: config.stateStreamUrl });
      if (cancelled) {
        nextDb.close();
        return;
      }
      setDb(nextDb);
      setReady(true);
    });

    return () => {
      cancelled = true;
      setDb((current) => {
        current?.close();
        return null;
      });
    };
  }, [baseUrl]);

  return (
    <QueryClientProvider client={queryClient}>
      <FlamecastContext.Provider value={client}>
        <FirelineDbContext.Provider value={db}>
          {ready ? children : <div className="p-4 text-sm text-muted-foreground">Connecting to Fireline…</div>}
        </FirelineDbContext.Provider>
      </FlamecastContext.Provider>
    </QueryClientProvider>
  );
}

export function useFlamecastClient(): FlamecastClient {
  const client = useContext(FlamecastContext);
  if (!client) throw new Error("useFlamecastClient must be used within <FlamecastProvider>");
  return client;
}

export function useFirelineDb(): FirelineDB {
  const db = useContext(FirelineDbContext);
  if (!db) throw new Error("useFirelineDb must be used within <FlamecastProvider>");
  return db;
}

async function fetchFirelineConfig(baseUrl: string) {
  const response = await fetch(`${baseUrl.replace(/\/$/, "")}/api/fireline-config`, {
    headers: { accept: "application/json" },
  });
  if (!response.ok) {
    throw new Error(`request failed (${response.status}) for /api/fireline-config`);
  }
  return (await response.json()) as {
    firelineUrl: string;
    stateStreamUrl: string;
    workspaceRoot: string;
  };
}
