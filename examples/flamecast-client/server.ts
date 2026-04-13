import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { randomUUID } from "node:crypto";
import { promises as fs } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createServer as createViteServer } from "vite";
import {
  agent,
  compose,
  db as openFirelineDb,
  type FirelineAgent,
  type Middleware,
  middleware,
  sandbox,
  type FirelineDB,
} from "@fireline/client";
import { SandboxAdmin } from "@fireline/client/admin";
import { approve, secretsProxy, trace } from "@fireline/client/middleware";
import { localPath } from "@fireline/client/resources";
import { type ChunkRow, type PermissionRow, type PromptTurnRow } from "@fireline/state";
import type {
  AgentSpawn,
  AgentTemplate,
  FilePreview,
  FileSystemEntry,
  FileSystemSnapshot,
  PendingPermission,
  PermissionResponseBody,
  QueuedMessage,
  RuntimeInfo,
  RuntimeInstance,
  Session,
} from "./ui/fireline-types.ts";
import { buildSessionLogs } from "./ui/lib/build-session-logs.js";

type RuntimeRecord = {
  typeName: string;
  instanceName: string;
  handle: FirelineAgent;
  status: "running" | "stopped" | "paused";
  createdAt: string;
  updatedAt: string;
};

type SessionRecord = {
  sessionId: string;
  sandboxId: string;
  handle: FirelineAgent;
  connection: Awaited<ReturnType<FirelineAgent["connect"]>>;
  runtimeInstance: string;
  agentTemplateId: string;
  agentName: string;
  spawn: AgentSpawn;
  cwd: string;
  startedAt: string;
  lastUpdatedAt: string;
  status: "active" | "killed";
  title?: string;
};

type ProviderName = "local" | "docker" | "microsandbox" | "anthropic";

const here = path.dirname(fileURLToPath(import.meta.url));
const workspaceRoot = path.resolve(process.env.FLAMECAST_WORKSPACE ?? process.cwd());
const firelineUrl = process.env.FIRELINE_URL ?? "http://127.0.0.1:4440";
const sharedStateStream = process.env.FLAMECAST_STATE_STREAM ?? `flamecast-client-${Date.now()}`;
const serverPort = Number(process.env.PORT ?? 3001);
const defaultRuntimeType: ProviderName = "local";
const defaultRuntimeInstance = process.env.FLAMECAST_DEFAULT_RUNTIME ?? "workspace";
const defaultSpawn = parseCommand(
  process.env.AGENT_COMMAND ?? "npx -y @anthropic-ai/claude-code-acp",
);
const sharedSecrets = process.env.ANTHROPIC_API_KEY
  ? { ANTHROPIC_API_KEY: { ref: "env:ANTHROPIC_API_KEY" } }
  : undefined;

const admin = new SandboxAdmin({ serverUrl: firelineUrl });
const runtimes = new Map<string, RuntimeRecord>();
const sessions = new Map<string, SessionRecord>();
const templates = new Map<string, AgentTemplate>();
const sessionSettings = new Map<string, { autoApprovePermissions: boolean }>();
const autoApproved = new Set<string>();
const messageQueue: QueuedMessage[] = [];
let nextMessageId = 1;
let globalSettings = { autoApprovePermissions: false };
let stateStreamUrl = "";
let stateDb: FirelineDB | null = null;

seedTemplates();
await ensureRuntime(defaultRuntimeInstance, defaultRuntimeType);

const vite =
  process.env.NODE_ENV === "production"
    ? null
    : await createViteServer({
        root: here,
        server: { middlewareMode: true },
        appType: "spa",
      });

const server = createServer(async (req, res) => {
  try {
    const url = new URL(req.url ?? "/", `http://${req.headers.host ?? "127.0.0.1"}`);
    if (url.pathname.startsWith("/api/")) {
      await handleApi(req, res, url);
      return;
    }
    if (vite) {
      vite.middlewares(req, res, (error: unknown) => {
        if (error) {
          res.statusCode = 500;
          res.end(String(error));
        }
      });
      return;
    }
    await serveStatic(res, url.pathname);
  } catch (error) {
    res.writeHead(500, { "content-type": "application/json" });
    res.end(JSON.stringify({ error: error instanceof Error ? error.message : String(error) }));
  }
});

server.listen(serverPort, () => {
  console.log(
    JSON.stringify(
      {
        example: "flamecast-client",
        firelineUrl,
        workspaceRoot,
        stateStreamUrl,
        url: `http://127.0.0.1:${serverPort}`,
      },
      null,
      2,
    ),
  );
});

async function handleApi(req: IncomingMessage, res: ServerResponse, url: URL) {
  const segments = url.pathname.split("/").filter(Boolean);
  const method = req.method ?? "GET";

  if (segments[1] === "fireline-config" && method === "GET") {
    return json(res, 200, {
      firelineUrl,
      stateStreamUrl,
      workspaceRoot,
    });
  }

  if (segments[1] === "settings") {
    if (method === "GET") {
      return json(res, 200, globalSettings);
    }
    if (method === "PATCH") {
      const patch = (await readJson(req)) as Partial<typeof globalSettings>;
      globalSettings = { ...globalSettings, ...patch };
      return json(res, 200, globalSettings);
    }
  }

  if (segments[1] === "agent-templates") {
    if (method === "GET" && segments.length === 2) {
      return json(res, 200, [...templates.values()]);
    }
    if (method === "POST" && segments.length === 2) {
      const body = (await readJson(req)) as {
        name: string;
        spawn: AgentSpawn;
        runtime?: AgentTemplate["runtime"];
        env?: Record<string, string>;
      };
      const created: AgentTemplate = {
        id: randomUUID(),
        name: body.name,
        spawn: body.spawn,
        runtime: body.runtime ?? { provider: defaultRuntimeType },
        env: body.env,
      };
      templates.set(created.id, created);
      return json(res, 201, created);
    }
    if (method === "PUT" && segments.length === 3) {
      const current = templates.get(segments[2]);
      if (!current) {
        return json(res, 404, { error: "template not found" });
      }
      const patch = (await readJson(req)) as {
        name?: string;
        spawn?: AgentSpawn;
        runtime?: Partial<AgentTemplate["runtime"]>;
        env?: Record<string, string>;
      };
      const updated: AgentTemplate = {
        ...current,
        ...(patch.name ? { name: patch.name } : {}),
        ...(patch.spawn ? { spawn: patch.spawn } : {}),
        ...(patch.env ? { env: patch.env } : {}),
        ...(patch.runtime ? { runtime: { ...current.runtime, ...patch.runtime } } : {}),
      };
      templates.set(updated.id, updated);
      return json(res, 200, updated);
    }
  }

  if (segments[1] === "runtimes") {
    if (method === "GET" && segments.length === 2) {
      return json(res, 200, listRuntimes());
    }
    if (method === "POST" && segments.length === 4 && segments[3] === "start") {
      const typeName = segments[2];
      const body = (await readJson(req)) as { name?: string };
      const instanceName = body.name?.trim() || `${typeName}-${Math.floor(Date.now() / 1000)}`;
      const runtime = await ensureRuntime(instanceName, typeName);
      return json(res, 201, toRuntimeInstance(runtime));
    }
    if (segments.length >= 3) {
      const instanceName = decodeURIComponent(segments[2]);
      if (method === "POST" && segments[3] === "stop") {
        const runtime = requireRuntime(instanceName);
        runtime.status = "stopped";
        runtime.updatedAt = nowIso();
        return json(res, 200, { ok: true });
      }
      if (method === "POST" && segments[3] === "pause") {
        const runtime = requireRuntime(instanceName);
        runtime.status = "paused";
        runtime.updatedAt = nowIso();
        return json(res, 200, { ok: true });
      }
      if (method === "DELETE" && segments.length === 3) {
        const runtime = requireRuntime(instanceName);
        await runtime.handle.destroy();
        runtimes.delete(instanceName);
        return json(res, 200, { ok: true });
      }
      if (method === "GET" && segments[3] === "files") {
        const pathParam = url.searchParams.get("path");
        return json(res, 200, await filePreview(resolvePath(pathParam, workspaceRoot)));
      }
      if (method === "GET" && segments[3] === "fs" && segments[4] === "snapshot") {
        return json(
          res,
          200,
          await snapshotDirectory(
            workspaceRoot,
            resolvePath(url.searchParams.get("path"), workspaceRoot),
            url.searchParams.get("showAllFiles") === "true",
          ),
        );
      }
      if (method === "GET" && segments[3] === "fs" && segments[4] === "commands") {
        return json(res, 200, slashCommands());
      }
      if (method === "GET" && segments[3] === "fs" && segments[4] === "git" && segments[5] === "branches") {
        return json(res, 200, await gitBranches(resolvePath(url.searchParams.get("path"), workspaceRoot)));
      }
      if (method === "GET" && segments[3] === "fs" && segments[4] === "git" && segments[5] === "worktrees") {
        return json(res, 200, await gitWorktrees(resolvePath(url.searchParams.get("path"), workspaceRoot)));
      }
      if (method === "POST" && segments[3] === "fs" && segments[4] === "git" && segments[5] === "worktrees") {
        const body = (await readJson(req)) as { name: string; path?: string; branch?: string };
        return json(res, 200, {
          path: body.path ?? workspaceRoot,
          message: `TODO: provision worktree '${body.name}' via Fireline-mounted workspace`,
        });
      }
    }
  }

  if (segments[1] === "agents") {
    if (method === "GET" && segments.length === 2) {
      return json(res, 200, await listSessions());
    }
    if (method === "POST" && segments.length === 2) {
      const body = (await readJson(req)) as {
        cwd?: string;
        agentTemplateId?: string;
        runtimeInstance?: string;
        name?: string;
      };
      const created = await createSession(body);
      return json(res, 201, created);
    }
    if (segments.length >= 3) {
      const sessionId = decodeURIComponent(segments[2]);
      if (method === "GET" && segments.length === 3) {
        return json(res, 200, await buildSessionResponse(requireSession(sessionId), {
          includeFileSystem: url.searchParams.get("includeFileSystem") === "true",
          showAllFiles: url.searchParams.get("showAllFiles") === "true",
        }));
      }
      if (method === "DELETE" && segments.length === 3) {
        await terminateSession(sessionId);
        return json(res, 200, { ok: true });
      }
      if (method === "POST" && segments[3] === "prompts") {
        const body = (await readJson(req)) as { text: string };
        await promptSession(sessionId, body.text);
        return json(res, 200, { ok: true });
      }
      if (method === "GET" && segments[3] === "status") {
        return json(res, 200, await sessionStatus(sessionId));
      }
      if (method === "POST" && segments[3] === "permissions" && segments[4]) {
        const body = (await readJson(req)) as PermissionResponseBody;
        const permission = await resolveSessionPermission(sessionId, decodeURIComponent(segments[4]), body);
        return json(res, 200, permission);
      }
      if (method === "GET" && segments[3] === "settings") {
        return json(res, 200, sessionSettings.get(sessionId) ?? { autoApprovePermissions: false });
      }
      if (method === "PATCH" && segments[3] === "settings") {
        const patch = (await readJson(req)) as Partial<{ autoApprovePermissions: boolean }>;
        const current = sessionSettings.get(sessionId) ?? { autoApprovePermissions: false };
        const next = { ...current, ...patch };
        sessionSettings.set(sessionId, next);
        return json(res, 200, next);
      }
      if (method === "GET" && segments[3] === "files") {
        const record = requireSession(sessionId);
        const preview = await previewSessionFile(record, url.searchParams.get("path") ?? "");
        return json(res, 200, preview);
      }
      if (method === "GET" && segments[3] === "fs" && segments[4] === "snapshot") {
        const record = requireSession(sessionId);
        const root = normalizeSessionRoot(record.cwd);
        return json(
          res,
          200,
          await snapshotDirectory(
            root,
            resolvePath(url.searchParams.get("path"), root),
            url.searchParams.get("showAllFiles") === "true",
          ),
        );
      }
      if (method === "GET" && segments[3] === "commands") {
        return json(res, 200, slashCommands());
      }
    }
  }

  if (segments[1] === "message-queue") {
    if (method === "GET" && segments.length === 2) {
      return json(res, 200, messageQueue);
    }
    if (method === "POST" && segments.length === 2) {
      const body = (await readJson(req)) as Omit<QueuedMessage, "id" | "createdAt" | "sentAt" | "status">;
      const queued: QueuedMessage = {
        id: nextMessageId++,
        createdAt: nowIso(),
        sentAt: null,
        status: "pending",
        ...body,
      };
      messageQueue.unshift(queued);
      return json(res, 201, queued);
    }
    if (method === "DELETE" && segments.length === 2) {
      messageQueue.splice(0, messageQueue.length);
      return json(res, 200, { ok: true });
    }
    if (segments.length >= 3) {
      const id = Number(segments[2]);
      if (method === "DELETE" && segments.length === 3) {
        const index = messageQueue.findIndex((item) => item.id === id);
        if (index >= 0) {
          messageQueue.splice(index, 1);
        }
        return json(res, 200, { ok: true });
      }
      if (method === "POST" && segments[3] === "send") {
        const item = messageQueue.find((entry) => entry.id === id);
        if (!item || !item.sessionId) {
          return json(res, 404, { error: "queue item not found" });
        }
        await promptSession(item.sessionId, item.text);
        item.status = "sent";
        item.sentAt = nowIso();
        return json(res, 200, { ok: true });
      }
    }
  }

  return json(res, 404, { error: "not found" });
}

async function ensureRuntime(instanceName: string, typeName: string): Promise<RuntimeRecord> {
  const existing = runtimes.get(instanceName);
  if (existing) {
    existing.status = "running";
    existing.updatedAt = nowIso();
    return existing;
  }

  const handle = await provisionSandbox({
    spawn: defaultSpawn,
    provider: typeName as ProviderName,
    labels: {
      demo: "flamecast-client",
      kind: "runtime",
      runtime: typeName,
      instance: instanceName,
    },
  });

  await ensureStateDb(handle.state.url);

  const runtime: RuntimeRecord = {
    typeName,
    instanceName,
    handle,
    status: "running",
    createdAt: nowIso(),
    updatedAt: nowIso(),
  };
  runtimes.set(instanceName, runtime);
  return runtime;
}

async function createSession(body: {
  cwd?: string;
  agentTemplateId?: string;
  runtimeInstance?: string;
  name?: string;
}): Promise<Session> {
  const template = resolveTemplate(body.agentTemplateId);
  const runtimeInstance = body.runtimeInstance ?? defaultRuntimeInstance;
  await ensureRuntime(runtimeInstance, template.runtime.provider || defaultRuntimeType);

  const handle = await provisionSandbox({
    spawn: template.spawn,
    provider: (template.runtime.provider ?? defaultRuntimeType) as ProviderName,
    envVars: template.env,
    image: template.runtime.image,
    labels: {
      demo: "flamecast-client",
      kind: "session",
      runtime: runtimeInstance,
      template: template.id,
    },
  });
  await ensureStateDb(handle.state.url);

  const connection = await handle.connect(`flamecast-session-${Date.now()}`);
  const cwd = sanitizeCwd(body.cwd ?? workspaceRoot);
  const session = await connection.newSession({ cwd, mcpServers: [] });

  const record: SessionRecord = {
    sessionId: session.sessionId,
    sandboxId: handle.id,
    handle,
    connection,
    runtimeInstance,
    agentTemplateId: template.id,
    agentName: body.name?.trim() || template.name,
    spawn: template.spawn,
    cwd,
    startedAt: nowIso(),
    lastUpdatedAt: nowIso(),
    status: "active",
  };

  sessions.set(record.sessionId, record);
  return buildSessionResponse(record, { includeFileSystem: false, showAllFiles: false });
}

async function promptSession(sessionId: string, text: string) {
  const record = requireSession(sessionId);
  await record.connection.prompt({
    sessionId,
    prompt: [{ type: "text", text }],
  });
  if (!record.title) {
    record.title = shortTitle(text);
  }
  record.lastUpdatedAt = nowIso();
}

async function terminateSession(sessionId: string) {
  const record = requireSession(sessionId);
  record.status = "killed";
  record.lastUpdatedAt = nowIso();
  await record.handle.destroy();
  await record.connection.close();
}

async function resolveSessionPermission(
  sessionId: string,
  requestId: string,
  body: PermissionResponseBody,
) {
  const record = requireSession(sessionId);
  await record.handle.resolvePermission(sessionId, requestId, {
    allow: "optionId" in body,
    resolvedBy: "flamecast-client",
  });
  return { ok: true };
}

async function previewSessionFile(record: SessionRecord, relativePath: string): Promise<FilePreview> {
  const absolutePath = resolvePath(relativePath, normalizeSessionRoot(record.cwd));
  return filePreview(absolutePath, relativePath);
}

async function listSessions(): Promise<Session[]> {
  const responses = await Promise.all(
    [...sessions.values()].map((record) =>
      buildSessionResponse(record, { includeFileSystem: false, showAllFiles: false }),
    ),
  );
  return responses.sort((left, right) => Date.parse(right.lastUpdatedAt) - Date.parse(left.lastUpdatedAt));
}

function listRuntimes(): RuntimeInfo[] {
  const groups = new Map<string, RuntimeInstance[]>();
  for (const runtime of runtimes.values()) {
    const current = groups.get(runtime.typeName) ?? [];
    current.push(toRuntimeInstance(runtime));
    groups.set(runtime.typeName, current);
  }
  return [...groups.entries()].map(([typeName, instances]) => ({
    typeName,
    onlyOne: false,
    instances: instances.sort((left, right) => left.name.localeCompare(right.name)),
  }));
}

async function buildSessionResponse(
  record: SessionRecord,
  options: { includeFileSystem: boolean; showAllFiles: boolean },
): Promise<Session> {
  const turns = sessionTurns(record.sessionId);
  const permissions = sessionPermissions(record.sessionId);
  const chunks = sessionChunks(turns);
  const logs = buildSessionLogs(turns, chunks, permissions);
  const title = record.title ?? turns.find((turn) => turn.text)?.text ?? record.agentName;
  const pendingPermission = permissions.find((permission) => permission.state === "pending");
  const queueItems = messageQueue
    .filter((item) => item.sessionId === record.sessionId && item.status === "pending")
    .map((item, index) => ({
      queueId: String(item.id),
      text: item.text,
      enqueuedAt: item.createdAt,
      position: index + 1,
    }));
  const fileSystem = options.includeFileSystem
    ? await snapshotDirectory(
        normalizeSessionRoot(record.cwd),
        normalizeSessionRoot(record.cwd),
        options.showAllFiles,
      )
    : null;

  return {
    id: record.sessionId,
    sandboxId: record.sandboxId,
    agentName: record.agentName,
    spawn: record.spawn,
    startedAt: record.startedAt,
    lastUpdatedAt: record.lastUpdatedAt,
    status: record.status,
    logs,
    pendingPermission: pendingPermission ? toPendingPermission(pendingPermission) : null,
    fileSystem,
    promptQueue: {
      processing: turns.some((turn) => turn.state === "queued" || turn.state === "active"),
      paused: false,
      items: queueItems,
      size: queueItems.length,
    },
    websocketUrl: record.handle.acp.url,
    runtime: record.runtimeInstance,
    cwd: record.cwd,
    title,
    acpUrl: record.handle.acp.url,
    stateStreamUrl: record.handle.state.url,
  };
}

async function sessionStatus(sessionId: string) {
  const turns = sessionTurns(sessionId);
  const permissions = sessionPermissions(sessionId);
  return {
    processing: turns.some((turn) => turn.state === "queued" || turn.state === "active"),
    pendingPermission: permissions.some((permission) => permission.state === "pending"),
  };
}

async function ensureStateDb(url: string) {
  if (stateDb) {
    return;
  }
  stateStreamUrl = url;
  stateDb = await openFirelineDb({ stateStreamUrl: url });
  stateDb.permissions.subscribeChanges(() => {
    void autoApprovePendingPermissions();
  });
}

async function autoApprovePendingPermissions() {
  if (!stateDb || !stateStreamUrl) {
    return;
  }
  for (const permission of stateDb.permissions.toArray) {
    if (permission.state !== "pending") {
      continue;
    }
    const key = `${permission.sessionId}:${permission.requestId}`;
    if (autoApproved.has(key)) {
      continue;
    }
    const setting = sessionSettings.get(permission.sessionId) ?? { autoApprovePermissions: false };
    if (!globalSettings.autoApprovePermissions && !setting.autoApprovePermissions) {
      continue;
    }
    autoApproved.add(key);
    const session = sessions.get(permission.sessionId);
    if (session) {
      await session.handle.resolvePermission(permission.sessionId, permission.requestId, {
        allow: true,
        resolvedBy: "flamecast-client:auto-approve",
      });
      continue;
    }
  }
}

async function provisionSandbox(options: {
  spawn: AgentSpawn;
  provider: ProviderName;
  envVars?: Record<string, string>;
  image?: string;
  model?: string;
  labels: Record<string, string>;
}) {
  const resources =
    options.provider === "local" ? [localPath(workspaceRoot, workspaceRoot, false)] : [];
  const middlewareChain: Middleware[] = [trace(), approve({ scope: "tool_calls" })];
  if (sharedSecrets) {
    middlewareChain.push(secretsProxy(sharedSecrets));
  }

  return compose(
    buildSandboxConfig({
      provider: options.provider,
      image: options.image,
      model: options.model,
      resources,
      envVars: options.envVars,
      labels: options.labels,
    }),
    middleware(middlewareChain),
    agent([options.spawn.command, ...options.spawn.args]),
  ).start({
    serverUrl: firelineUrl,
    name: `${options.labels.kind}-${options.labels.instance ?? options.labels.template ?? randomUUID()}`,
    stateStream: sharedStateStream,
  });
}

function buildSandboxConfig(options: {
  provider: ProviderName;
  resources: ReturnType<typeof localPath>[];
  envVars?: Record<string, string>;
  labels: Record<string, string>;
  image?: string;
  model?: string;
}) {
  switch (options.provider) {
    case "docker":
      return sandbox({
        provider: "docker",
        resources: options.resources,
        envVars: options.envVars,
        labels: options.labels,
        ...(options.image ? { image: options.image } : {}),
      });
    case "anthropic":
      return sandbox({
        provider: "anthropic",
        resources: options.resources,
        envVars: options.envVars,
        labels: options.labels,
        ...(options.model ? { model: options.model } : {}),
      });
    case "microsandbox":
      return sandbox({
        provider: "microsandbox",
        resources: options.resources,
        envVars: options.envVars,
        labels: options.labels,
      });
    case "local":
    default:
      return sandbox({
        provider: "local",
        resources: options.resources,
        envVars: options.envVars,
        labels: options.labels,
      });
  }
}

function resolveTemplate(templateId?: string): AgentTemplate {
  const template = templateId ? templates.get(templateId) : [...templates.values()][0];
  if (!template) {
    throw new Error("no agent templates are registered");
  }
  return template;
}

function requireRuntime(instanceName: string): RuntimeRecord {
  const runtime = runtimes.get(instanceName);
  if (!runtime) {
    throw new Error(`runtime '${instanceName}' not found`);
  }
  return runtime;
}

function requireSession(sessionId: string): SessionRecord {
  const session = sessions.get(sessionId);
  if (!session) {
    throw new Error(`session '${sessionId}' not found`);
  }
  return session;
}

function toRuntimeInstance(runtime: RuntimeRecord): RuntimeInstance {
  return {
    name: runtime.instanceName,
    typeName: runtime.typeName,
    status: runtime.status,
    sandboxId: runtime.handle.id,
    websocketUrl: runtime.handle.acp.url,
    acpUrl: runtime.handle.acp.url,
    stateStreamUrl: runtime.handle.state.url,
  };
}

function sessionTurns(sessionId: string): PromptTurnRow[] {
  if (!stateDb) {
    return [];
  }
  return [...stateDb.promptRequests.toArray]
    .filter((turn) => turn.sessionId === sessionId)
    .sort((left, right) => left.startedAt - right.startedAt);
}

function sessionPermissions(sessionId: string): PermissionRow[] {
  if (!stateDb) {
    return [];
  }
  return [...stateDb.permissions.toArray]
    .filter((permission) => permission.sessionId === sessionId)
    .sort((left, right) => left.createdAt - right.createdAt);
}

function sessionChunks(turns: PromptTurnRow[]): ChunkRow[] {
  if (!stateDb) {
    return [];
  }
  const requestIds = new Set(turns.map((turn) => turn.requestId));
  return [...stateDb.chunks.toArray]
    .filter((chunk) => requestIds.has(chunk.requestId))
    .sort((left, right) => left.createdAt - right.createdAt);
}

function toPendingPermission(permission: PermissionRow): PendingPermission {
  return {
    requestId: permission.requestId,
    toolCallId: permission.toolCallId ?? "",
    title: permission.title ?? "Permission required",
    options:
      permission.options?.map((option) => ({
        optionId: option.optionId,
        name: option.name,
        kind: option.kind,
      })) ?? [],
  };
}

async function snapshotDirectory(
  root: string,
  currentPath: string,
  showAllFiles: boolean,
): Promise<FileSystemSnapshot> {
  const resolvedRoot = path.resolve(root);
  const resolvedCurrent = path.resolve(currentPath);
  const safeCurrent = resolvedCurrent.startsWith(resolvedRoot) ? resolvedCurrent : resolvedRoot;
  const entries = await fs.readdir(safeCurrent, { withFileTypes: true });
  const visible = showAllFiles ? entries : entries.filter((entry) => !entry.name.startsWith("."));

  return {
    root: resolvedRoot,
    path: safeCurrent,
    gitPath: await detectGitRoot(safeCurrent),
    entries: await Promise.all(
      visible.slice(0, 200).map(async (entry) => ({
        path: entry.name,
        type: entry.isDirectory()
          ? "directory"
          : entry.isFile()
            ? "file"
            : entry.isSymbolicLink()
              ? "symlink"
              : "other",
        ...(entry.isDirectory() ? await maybeGitInfo(path.join(safeCurrent, entry.name)) : {}),
      })),
    ),
    truncated: visible.length > 200,
    maxEntries: 200,
  };
}

async function filePreview(absolutePath: string, displayPath = absolutePath): Promise<FilePreview> {
  const content = await fs.readFile(absolutePath, "utf8");
  const maxChars = 40_000;
  return {
    path: displayPath,
    content: content.slice(0, maxChars),
    truncated: content.length > maxChars,
    maxChars,
  };
}

async function maybeGitInfo(entryPath: string): Promise<Partial<FileSystemEntry>> {
  const gitRoot = await detectGitRoot(entryPath);
  if (!gitRoot || gitRoot !== entryPath) {
    return {};
  }
  const branch = await gitCurrentBranch(entryPath);
  return branch ? { git: { branch } } : {};
}

async function gitBranches(currentPath: string) {
  const gitRoot = await detectGitRoot(currentPath);
  if (!gitRoot) {
    return { branches: [] };
  }
  const output = await runCommand("git", ["-C", gitRoot, "branch", "--format=%(refname:short)|%(objectname)|%(HEAD)"]);
  return {
    branches: output
      .split("\n")
      .filter(Boolean)
      .map((line) => {
        const [name, sha, head] = line.split("|");
        return {
          name,
          sha,
          current: head === "*",
          remote: false,
        };
      }),
  };
}

async function gitWorktrees(currentPath: string) {
  const gitRoot = await detectGitRoot(currentPath);
  if (!gitRoot) {
    return { worktrees: [] };
  }
  const output = await runCommand("git", ["-C", gitRoot, "worktree", "list", "--porcelain"]);
  const worktrees: Array<{ path: string; sha?: string; branch?: string; detached?: boolean }> = [];
  let current: { path: string; sha?: string; branch?: string; detached?: boolean } | null = null;
  for (const line of output.split("\n")) {
    if (!line.trim()) {
      if (current) {
        worktrees.push(current);
      }
      current = null;
      continue;
    }
    if (line.startsWith("worktree ")) {
      current = { path: line.slice("worktree ".length) };
      continue;
    }
    if (!current) {
      continue;
    }
    if (line.startsWith("HEAD ")) {
      current.sha = line.slice("HEAD ".length);
    } else if (line.startsWith("branch refs/heads/")) {
      current.branch = line.slice("branch refs/heads/".length);
    } else if (line === "detached") {
      current.detached = true;
    }
  }
  if (current) {
    worktrees.push(current);
  }
  return { worktrees };
}

function slashCommands() {
  return [
    { name: "/review", description: "Review the current workspace changes" },
    { name: "/plan", description: "Plan the next implementation step" },
    { name: "/test", description: "Run the relevant verification command" },
  ];
}

async function serveStatic(res: ServerResponse, pathname: string) {
  const filePath =
    pathname === "/"
      ? path.join(here, "dist", "index.html")
      : path.join(here, "dist", pathname.replace(/^\/+/, ""));
  const safePath = filePath.startsWith(path.join(here, "dist")) ? filePath : path.join(here, "dist", "index.html");
  try {
    const bytes = await fs.readFile(safePath);
    res.writeHead(200, { "content-type": contentType(safePath) });
    res.end(bytes);
  } catch {
    const indexHtml = await fs.readFile(path.join(here, "index.html"));
    res.writeHead(200, { "content-type": "text/html; charset=utf-8" });
    res.end(indexHtml);
  }
}

function seedTemplates() {
  const template: AgentTemplate = {
    id: "claude-code-local",
    name: "Claude Code",
    spawn: defaultSpawn,
    runtime: { provider: defaultRuntimeType },
  };
  templates.set(template.id, template);
}

function sanitizeCwd(value: string): string {
  const resolved = path.resolve(value);
  return resolved.startsWith(workspaceRoot) ? resolved : workspaceRoot;
}

function normalizeSessionRoot(cwd: string): string {
  return sanitizeCwd(cwd || workspaceRoot);
}

function resolvePath(input: string | null, base: string): string {
  if (!input) {
    return base;
  }
  const resolved = path.resolve(input.startsWith("/") ? input : path.join(base, input));
  return resolved.startsWith(workspaceRoot) ? resolved : base;
}

function nowIso(): string {
  return new Date().toISOString();
}

function shortTitle(text: string): string {
  return text.replace(/\s+/g, " ").trim().slice(0, 80);
}

function parseCommand(command: string): AgentSpawn {
  const [bin, ...args] = command.split(/\s+/).filter(Boolean);
  if (!bin) {
    throw new Error("AGENT_COMMAND must not be empty");
  }
  return { command: bin, args };
}

async function readJson(req: IncomingMessage): Promise<unknown> {
  const chunks: Uint8Array[] = [];
  for await (const chunk of req) {
    chunks.push(typeof chunk === "string" ? Buffer.from(chunk) : chunk);
  }
  if (chunks.length === 0) {
    return {};
  }
  return JSON.parse(Buffer.concat(chunks).toString("utf8"));
}

function json(res: ServerResponse, status: number, body: unknown) {
  res.writeHead(status, { "content-type": "application/json" });
  res.end(JSON.stringify(body));
}

function contentType(filePath: string): string {
  switch (path.extname(filePath)) {
    case ".js":
      return "text/javascript; charset=utf-8";
    case ".css":
      return "text/css; charset=utf-8";
    case ".svg":
      return "image/svg+xml";
    case ".woff2":
      return "font/woff2";
    case ".webp":
      return "image/webp";
    default:
      return "text/html; charset=utf-8";
  }
}

async function detectGitRoot(currentPath: string): Promise<string | undefined> {
  let cursor = currentPath;
  while (cursor.startsWith(workspaceRoot)) {
    try {
      const stats = await fs.stat(path.join(cursor, ".git"));
      if (stats.isDirectory() || stats.isFile()) {
        return cursor;
      }
    } catch {
      // keep walking
    }
    const parent = path.dirname(cursor);
    if (parent === cursor) {
      break;
    }
    cursor = parent;
  }
  return undefined;
}

async function gitCurrentBranch(repoPath: string): Promise<string | undefined> {
  try {
    const output = await runCommand("git", ["-C", repoPath, "branch", "--show-current"]);
    return output.trim() || undefined;
  } catch {
    return undefined;
  }
}

async function runCommand(command: string, args: string[]): Promise<string> {
  const child = await import("node:child_process");
  return await new Promise<string>((resolve, reject) => {
    child.execFile(command, args, { cwd: workspaceRoot }, (error, stdout) => {
      if (error) {
        reject(error);
        return;
      }
      resolve(stdout.trim());
    });
  });
}
