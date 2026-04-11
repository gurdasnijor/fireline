# 13: Distributed Runtime Fabric Foundation

This document now serves as a compatibility pointer.

The old single-doc slice 13 has been split into a folder because it was too
large to use as a direct Codex handoff target.

Use these docs instead:

- [`13-distributed-runtime-fabric/README.md`](./13-distributed-runtime-fabric/README.md)
  Umbrella, sequence, and scope boundary for the whole slice family.
- [`13-distributed-runtime-fabric/phase-0-runtime-host-and-peer-registry-refactor.md`](./13-distributed-runtime-fabric/phase-0-runtime-host-and-peer-registry-refactor.md)
  Pure prerequisite refactor, zero behavior change.
- [`13-distributed-runtime-fabric/13a-control-plane-runtime-api-and-external-durable-stream-bootstrap.md`](./13-distributed-runtime-fabric/13a-control-plane-runtime-api-and-external-durable-stream-bootstrap.md)
  First real Codex handoff target.
- [`13-distributed-runtime-fabric/13b-docker-provider-and-mixed-topology.md`](./13-distributed-runtime-fabric/13b-docker-provider-and-mixed-topology.md)
  Docker and mixed-topology expansion after `13a`.

If you are deciding whether slice 13 is ready to hand off, the answer is:

- hand off one child doc
- do not hand off the whole umbrella at once
