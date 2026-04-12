import { useQuery } from "@tanstack/react-query";
import type { SandboxDescriptor } from "@fireline/client";
import { useFlamecastClient } from "../provider.js";
import type { RuntimeInfo, RuntimeInstance } from "../fireline-types.js";

export function useRuntimes() {
  const client = useFlamecastClient();
  return useQuery({
    queryKey: ["runtimes"],
    queryFn: async () => toRuntimeInfo(await client.admin.list({ demo: "flamecast-client", kind: "runtime" })),
    refetchInterval: 30_000,
  });
}

function toRuntimeInfo(descriptors: SandboxDescriptor[]): RuntimeInfo[] {
  const groups = new Map<string, RuntimeInstance[]>();

  for (const descriptor of descriptors) {
    const typeName = descriptor.labels.runtime ?? descriptor.provider;
    const instanceName = descriptor.labels.instance ?? descriptor.id;
    const current = groups.get(typeName) ?? [];
    current.push({
      name: instanceName,
      typeName,
      status: mapStatus(descriptor.status),
      sandboxId: descriptor.id,
      websocketUrl: descriptor.acp.url,
      acpUrl: descriptor.acp.url,
      stateStreamUrl: descriptor.state.url,
    });
    groups.set(typeName, current);
  }

  return [...groups.entries()]
    .map(([typeName, instances]) => ({
      typeName,
      onlyOne: false,
      instances: instances.sort((left, right) => left.name.localeCompare(right.name)),
    }))
    .sort((left, right) => left.typeName.localeCompare(right.typeName));
}

function mapStatus(status: string): RuntimeInstance["status"] {
  switch (status) {
    case "stopped":
      return "stopped";
    case "broken":
      return "paused";
    default:
      return "running";
  }
}
