# Product Backlog

> Related:
> - [`index.md`](./index.md)
> - [`priorities.md`](./priorities.md)
> - [`roadmap-alignment.md`](./roadmap-alignment.md)
> - [`../execution/12-programmable-topology-first-mover.md`](../execution/12-programmable-topology-first-mover.md)
> - [`../execution/13-distributed-runtime-fabric/README.md`](../execution/13-distributed-runtime-fabric/README.md)

## Purpose

This table is the bridge between the product vision and future execution docs.

It includes both:

- `slice` items, which should usually become execution docs
- `spike` items, which are research or proof-oriented and should sharpen a
  later slice

## Backlog Table

| ID | Type | Theme | Product Pillar | User Workflow Unlocked | Depends On | Notes |
|---|---|---|---|---|---|---|
| `13a` | slice | Control-plane runtime API + external durable-stream bootstrap | provider-neutral runtime fabric | Observe and manage one coherent control-plane-backed local runtime fabric | `12` | First practical cut of the environment-level runtime contract; `LocalProvider` only |
| `13b` | slice | Docker provider + mixed topology | provider-neutral runtime fabric | Observe and manage one coherent local + Docker runtime fabric | `13a` | Adds non-local provider proof and shared durable-streams mixed topology |
| `14` | slice | Session product surface | durable sessions | List, inspect, reopen, and reason about long-running runs | `13a` | Should expose sessions and runs as clearer objects |
| `15` | slice | Capability profiles | portable capability profiles | Reuse MCPs, policies, skills, and defaults across runs and runtimes | `13a` | Strong place to define profile shape |
| `15a` | spike | `agent.pw` integration seam | portable capability profiles | Resolve credentials just in time instead of injecting raw secrets into runtimes | `15` | Define how profiles reference credential paths and auth scopes |
| `16` | slice | Approval gates + out-of-band service | reusable conductor extensions | Let long-running runs pause on gated actions and resume after approval later | `12`, `13a` | Strong product differentiation for background agents |
| `16a` | spike | Permission queue and service model | durable sessions | Decide durable record shapes for paused approvals and resumptions | `16` | Define records, statuses, and resume semantics |
| `17` | slice | Workspace model | portable workspaces | Start a run from local path, git ref, or snapshot with stable workspace identity | `13a` | Needed before remote execution feels coherent |
| `17a` | spike | Workspace sync strategies | portable workspaces | Choose between bind, snapshot, rsync, or provider-specific sync | `17` | Important for local-to-remote move story |
| `18` | slice | ACP agent augmentation story | reusable conductor extensions | Add audit, context, approvals, and lineage around ACP-native or ACP-adapted agents without replacing them | `12`, `13a` | Fireline as augmentation layer rather than replacement |
| `18a` | spike | ACP augmentation capability matrix | reusable conductor extensions | Define which capabilities require only baseline ACP and which need richer downstream support such as `session/load` | `18` | Keep this capability-focused, not brand-specific |
| `19` | slice | Browser control-plane session UX | durable sessions | Resume and observe runs from browser or mobile-friendly UI | `14`, `16` | Product-facing proof that sessions are real |
| `20` | slice | Background workflow entrypoints | durable sessions | Start durable runs from GitHub, Slack, or scheduled triggers | `14`, `15`, `16` | Strong path for non-interactive agents |
| `21` | slice | Recording and replay | reusable conductor extensions | Reproduce prior runs, build fixtures, and inspect failures durably | `12`, `14` | Valuable for trust, debugging, and eval |
| `22` | slice | Cloudflare provider | provider-neutral runtime fabric | Run Fireline-managed sessions on Cloudflare-backed runtimes | `13b` | Provider expansion, not architecture driver |

## Selection Guidance

Use this table with the slice-selection rule in
[`roadmap-alignment.md`](./roadmap-alignment.md).

In practice:

- choose slices that strengthen one product pillar clearly
- prefer slices that unlock a visible user workflow
- use spikes to reduce ambiguity before opening broad execution docs
