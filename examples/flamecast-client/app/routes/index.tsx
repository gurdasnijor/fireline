// @ts-nocheck
import { createFileRoute, Link, useNavigate } from "@tanstack/react-router";
import {
  useRuntimes,
  useAgentTemplates,
  useCreateSession,
  useStartRuntime,
  useRuntimeFileSystem,
  useFlamecastClient,
  useMessageQueue,
} from "@flamecast/ui";
import { useDefaultAgentConfig } from "@/lib/default-agent-config-context";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { DirectoryPicker } from "@/components/directory-picker";
import { GitWorktreeMenu, useActiveBranch } from "@/components/git-worktree-picker";
import { SlashCommandInput } from "@/components/slash-command-input";
import { Skeleton } from "@/components/ui/skeleton";
import { ChevronDownIcon, FolderOpenIcon, LoaderCircleIcon, PlusIcon } from "lucide-react";
import { toast } from "sonner";
import { useCallback, useState } from "react";
import { useEnqueueMessage } from "@flamecast/ui";
import flamecastMascots from "@/assets/flamecast_mascots.webp";

export const Route = createFileRoute("/")({
  component: HomePage,
});

function HomePage() {
  return <DeveloperHomePage />;
}

// ─── Developer Home Page (full controls) ──────────────────────────────────────

function DeveloperHomePage() {
  const navigate = useNavigate();
  const { config } = useDefaultAgentConfig();
  const { data: runtimes, isLoading: runtimesLoading } = useRuntimes();
  const { data: templates, isLoading: templatesLoading } = useAgentTemplates();
  const { data: queue = [] } = useMessageQueue();

  // --- Runtime type selection (default: first) ---
  const defaultRuntime = runtimes?.[0]?.typeName ?? "";
  const [selectedRuntime, setSelectedRuntime] = useState<string>("");
  const activeRuntime = selectedRuntime || defaultRuntime;
  const runtimeInfo = runtimes?.find((rt) => rt.typeName === activeRuntime);
  const isMultiInstance = runtimeInfo ? !runtimeInfo.onlyOne : false;

  // --- Runtime instance selection (default: first running) ---
  const [selectedInstanceName, setSelectedInstanceName] = useState<string>("");
  const runningInstances = runtimeInfo?.instances.filter((i) => i.status === "running") ?? [];
  const stoppedInstances =
    runtimeInfo?.instances.filter((i) => i.status === "stopped" || i.status === "paused") ?? [];
  const activeInstance = isMultiInstance
    ? (runtimeInfo?.instances.find(
        (i) => i.name === selectedInstanceName && i.status === "running",
      ) ?? runningInstances[0])
    : undefined;
  const needsRunningInstance = isMultiInstance && runningInstances.length === 0;

  // --- Agent selection (default: from settings config, fallback to first) ---
  const matchingTemplates = templates?.filter((t) => t.runtime.provider === activeRuntime) ?? [];
  const defaultTemplate = matchingTemplates[0] ?? templates?.[0];
  const [selectedTemplateId, setSelectedTemplateId] = useState<string>(config.agentTemplateId);
  const activeTemplate = selectedTemplateId
    ? (templates?.find((t) => t.id === selectedTemplateId) ?? defaultTemplate)
    : defaultTemplate;

  // --- Working directory (default: from settings config) ---
  const [cwd, setCwd] = useState<string | undefined>(config.defaultDirectory || undefined);
  const [dirPickerOpen, setDirPickerOpen] = useState(false);

  // Resolve an instance name for the directory picker
  const pickerInstanceName =
    activeInstance?.name ??
    runtimeInfo?.instances.find((i) => i.status === "running")?.name ??
    activeRuntime;

  // --- Default root directory for the runtime ---
  const { data: defaultFsData } = useRuntimeFileSystem(pickerInstanceName);
  const defaultDir = defaultFsData?.root;

  // --- Git detection for selected directory ---
  const { data: cwdFsData } = useRuntimeFileSystem(pickerInstanceName, {
    enabled: !!cwd,
    path: cwd,
  });
  const gitPath = cwdFsData?.gitPath;
  const activeBranch = useActiveBranch(pickerInstanceName, gitPath, cwd ?? "");

  // --- Message queue ---
  const enqueueMutation = useEnqueueMessage({
    onSuccess: () => toast.success("Message queued"),
    onError: (err) => toast.error("Failed to queue message", { description: String(err.message) }),
  });

  // --- Mutations ---
  const client = useFlamecastClient();

  const startRuntimeMutation = useStartRuntime({
    onError: (err) => toast.error("Failed to start runtime", { description: String(err.message) }),
  });

  const createMutation = useCreateSession({
    onError: (err) => toast.error("Failed to create session", { description: String(err.message) }),
  });

  const isReady = !runtimesLoading && !templatesLoading && runtimes && runtimes.length > 0;
  const runtimeCount =
    runtimes?.flatMap((runtime) => runtime.instances).filter((instance) => instance.status === "running")
      .length ?? 0;
  const templateCount = templates?.length ?? 0;
  const queuedCount = queue.filter((item) => item.status === "pending").length;

  // Fetch slash commands for the selected directory
  const fetchCommands = useCallback(
    () =>
      client.rpc.runtimes[":instanceName"].fs.commands
        .$get({
          param: { instanceName: pickerInstanceName },
          query: { path: cwd ?? defaultDir },
        })
        .then((r) => (r.ok ? r.json() : []))
        .then((data) => (Array.isArray(data) ? data : [])),
    [client, pickerInstanceName, cwd, defaultDir],
  );

  const handleStartInstance = (instanceName?: string) => {
    startRuntimeMutation.mutate({ typeName: activeRuntime, name: instanceName });
  };

  const handleSend = (text: string) => {
    if (!text.trim() || !isReady || !activeTemplate) return;

    const templateName = activeTemplate.name;
    const templateId = activeTemplate.id;
    const runtimeName = activeRuntime;
    const instanceName = activeInstance?.name ?? activeRuntime;
    const dir = cwd ?? null;
    createMutation.mutate(
      {
        agentTemplateId: templateId,
        runtimeInstance: activeInstance?.name,
        cwd,
        agentName: templateName,
      },
      {
        onSuccess: (session) => {
          enqueueMutation.mutate({
            text,
            runtime: runtimeName,
            agent: templateName,
            agentTemplateId: templateId,
            directory: dir,
            sessionId: session.id,
          });
          void navigate({
            to: "/runtimes/$typeName/$instanceName",
            params: { typeName: runtimeName, instanceName },
            search: { sessionId: session.id },
          });
        },
      },
    );
  };

  const isBusy = startRuntimeMutation.isPending || createMutation.isPending;

  return (
    <div className="mx-auto flex min-h-0 w-full max-w-3xl flex-1 flex-col items-center justify-center gap-8 px-1">
      <div className="text-center">
        <div className="mx-auto mb-4 inline-flex items-center rounded-full border border-primary/20 bg-primary/8 px-3 py-1 text-[11px] font-medium uppercase tracking-[0.24em] text-primary">
          Reference Dashboard
        </div>
        <img
          src={flamecastMascots}
          alt="Flamecast mascots"
          className="mx-auto mb-6 w-full max-w-md"
        />
        <h1 className="text-3xl font-bold tracking-tight sm:text-4xl">Flamecast on Fireline</h1>
        <p className="mt-3 max-w-2xl text-sm leading-6 text-muted-foreground sm:text-base">
          Run the operator desk from one screen: start a runtime, open a session, queue the next
          customer follow-up, inspect the workspace, and resolve approvals without stitching
          together a custom admin panel.
        </p>
      </div>

      <div className="grid w-full gap-3 sm:grid-cols-3">
        <MetricCard
          label="Running runtimes"
          value={runtimeCount}
          detail="Live Fireline sandboxes ready for new sessions."
        />
        <MetricCard
          label="Agent templates"
          value={templateCount}
          detail="Reusable spawn profiles for the support floor."
        />
        <MetricCard
          label="Queued follow-ups"
          value={queuedCount}
          detail="Messages parked until the target session is ready."
        />
      </div>

      <div className="flex w-full flex-col gap-4 rounded-[28px] border border-border/70 bg-card/85 p-4 shadow-[0_24px_80px_-40px_rgba(97,54,21,0.45)] backdrop-blur sm:p-6">
        <SlashCommandInput
          fetchCommands={fetchCommands}
          onSend={handleSend}
          disabled={!isReady || needsRunningInstance || isBusy}
          placeholder={
            createMutation.isPending
              ? "Creating session…"
              : !isReady
                ? "Loading…"
                : needsRunningInstance
                  ? "Start a runtime instance first…"
                  : "Send a prompt or type / for commands…"
          }
        />
        {createMutation.isPending && (
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <LoaderCircleIcon className="size-4 animate-spin" />
            <span>Provisioning the session and queuing the first operator message…</span>
          </div>
        )}

        <div className="flex flex-wrap items-center gap-3">
          {/* Runtime type dropdown */}
          {runtimesLoading ? (
            <Skeleton className="h-6 w-28" />
          ) : runtimes && runtimes.length > 0 ? (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="ghost" size="sm" className="h-7 gap-1 px-2 text-xs">
                  <span className="text-muted-foreground">Runtime:</span>
                  {activeRuntime}
                  <ChevronDownIcon className="size-3 text-muted-foreground" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="start">
                {runtimes.map((rt) => (
                  <DropdownMenuItem
                    key={rt.typeName}
                    onSelect={() => {
                      setSelectedRuntime(rt.typeName);
                      setSelectedInstanceName("");
                      setSelectedTemplateId("");
                    }}
                  >
                    {rt.typeName}
                  </DropdownMenuItem>
                ))}
              </DropdownMenuContent>
            </DropdownMenu>
          ) : null}

          {/* Runtime instance dropdown (multi-instance only) */}
          {isReady && isMultiInstance ? (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="ghost" size="sm" className="h-7 gap-1 px-2 text-xs">
                  {startRuntimeMutation.isPending ? (
                    <LoaderCircleIcon className="size-3 animate-spin" />
                  ) : null}
                  <span className="text-muted-foreground">Instance:</span>
                  {startRuntimeMutation.isPending ? "Starting…" : (activeInstance?.name ?? "None")}
                  <ChevronDownIcon className="size-3 text-muted-foreground" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="start">
                {runningInstances.map((inst) => (
                  <DropdownMenuItem
                    key={inst.name}
                    onSelect={() => {
                      setSelectedInstanceName(inst.name);
                    }}
                  >
                    {inst.name}
                  </DropdownMenuItem>
                ))}
                {stoppedInstances.length > 0 && (
                  <>
                    <DropdownMenuSeparator />
                    {stoppedInstances.map((inst) => (
                      <DropdownMenuItem
                        key={inst.name}
                        onSelect={() => handleStartInstance(inst.name)}
                      >
                        {inst.name}
                        <span className="ml-auto text-[10px] text-muted-foreground">
                          {inst.status}
                        </span>
                      </DropdownMenuItem>
                    ))}
                  </>
                )}
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  onSelect={() => handleStartInstance()}
                  disabled={startRuntimeMutation.isPending}
                >
                  <PlusIcon className="size-3.5" />
                  Create new
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          ) : null}

          {/* Agent dropdown */}
          {runtimesLoading || templatesLoading ? (
            <Skeleton className="h-6 w-24" />
          ) : isReady ? (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="ghost" size="sm" className="h-7 gap-1 px-2 text-xs">
                  <span className="text-muted-foreground">Agent:</span>
                  {activeTemplate?.name ?? "None"}
                  <ChevronDownIcon className="size-3 text-muted-foreground" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="start">
                {matchingTemplates.length > 0
                  ? matchingTemplates.map((t) => (
                      <DropdownMenuItem
                        key={t.id}
                        onSelect={() => {
                          setSelectedTemplateId(t.id);
                        }}
                      >
                        {t.name}
                      </DropdownMenuItem>
                    ))
                  : templates && templates.length > 0
                    ? templates.map((t) => (
                        <DropdownMenuItem
                          key={t.id}
                          onSelect={() => {
                            setSelectedTemplateId(t.id);
                          }}
                        >
                          {t.name}
                          <span className="ml-auto text-[10px] text-muted-foreground">
                            {t.runtime.provider}
                          </span>
                        </DropdownMenuItem>
                      ))
                    : null}
                {(!templates || templates.length === 0) && (
                  <DropdownMenuItem disabled>No agents registered</DropdownMenuItem>
                )}
                <DropdownMenuSeparator />
                <DropdownMenuItem asChild>
                  <Link to="/agents">
                    <PlusIcon className="size-3.5" />
                    Create new
                  </Link>
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          ) : null}

          {/* Directory picker */}
          {isReady ? (
            <>
              <Button
                variant="ghost"
                size="sm"
                className="h-7 gap-1 px-2 text-xs"
                onClick={() => setDirPickerOpen(true)}
              >
                <FolderOpenIcon className="size-3" />
                <span className="text-muted-foreground">Dir:</span>
                <span className="max-w-32 truncate">{cwd ?? "default"}</span>
              </Button>
              <DirectoryPicker
                instanceName={pickerInstanceName}
                open={dirPickerOpen}
                onOpenChange={setDirPickerOpen}
                onSelect={(path) => setCwd(path)}
                initialPath={cwd}
              />
            </>
          ) : null}

          {/* Git branch dropdown */}
          {isReady && gitPath && activeBranch ? (
            <GitWorktreeMenu
              instanceName={pickerInstanceName}
              gitPath={gitPath}
              activeBranch={activeBranch}
              onSelect={(path) => setCwd(path)}
            />
          ) : null}
        </div>
      </div>
    </div>
  );
}

function MetricCard({
  label,
  value,
  detail,
}: {
  label: string;
  value: number;
  detail: string;
}) {
  return (
    <div className="rounded-[24px] border border-border/70 bg-background/80 px-4 py-4 text-left shadow-[0_12px_48px_-36px_rgba(97,54,21,0.4)]">
      <div className="text-[11px] font-medium uppercase tracking-[0.22em] text-muted-foreground">
        {label}
      </div>
      <div className="mt-2 text-3xl font-semibold tracking-tight text-foreground">{value}</div>
      <div className="mt-1 text-sm leading-5 text-muted-foreground">{detail}</div>
    </div>
  );
}
