# RFC: ACP Registry

> Status: design rationale
> Audience: engineers deciding whether Fireline should solve agent discovery with a shared catalog or with custom topology and install glue

Fireline wants a registry for one reason: naming.

Not for workflow state.
Not for deployment orchestration.
Not for a new control plane.

The registry exists so a user can say "run `pi-acp`" or "install `claude-acp`" and Fireline can resolve that name through a shared ACP catalog instead of making every project hand-roll its own lookup and install story.

That is the whole job.

## The Problem Fireline Is Solving

Without a registry layer, reusable agent identity is awkward in exactly the places users notice first:

- examples need hard-coded install instructions
- specs need explicit local command paths instead of a portable agent name
- every machine needs its own ad hoc mapping from logical agent name to binary
- teams end up encoding catalog behavior in docs, scripts, or bespoke topology glue

That is unnecessary friction for something the ACP ecosystem already has a natural shape for: a shared catalog of agent identities and install metadata.

Fireline does not need to invent a new discovery system to fix that. It only needs to consume the registry cleanly.

## The Decision

Fireline treats the ACP registry as a naming and install-discovery layer.

In practice, that means:

- the registry names agent identities such as `pi-acp` or `claude-acp`
- `fireline-agents add <id>` can install a registry-published ACP agent
- compose specs can use a narrow shorthand like `agent(['pi-acp'])`
- resolved agents still run through the normal command path once installed

The registry tells Fireline what an agent is called and how to obtain it.

It does not become the runtime, the workflow engine, or the deployment manager.

## Why A Catalog Is Better Than Custom Topology Glue

The alternative is to let every project solve naming locally.

That usually looks like one of these:

- a shell script that maps friendly names to binaries
- environment-specific install docs
- project-local wrappers for agent commands
- topology config that quietly doubles as an install catalog

All of those work in the short term. None of them scale cleanly across examples, demos, CI, or team handoff.

A shared registry is better because it pulls that concern back to its real level:

- agent identity is ecosystem-level metadata
- local installation is a user action or a narrow fallback
- runtime execution remains explicit once the binary exists

That lets Fireline be helpful without becoming magical.

## Why The Registry Is Not A Control Plane

This boundary matters.

A registry answers a read-mostly question: "what agent does this name refer to, and how do I install it?"

A control plane answers operational questions:

- where is this thing running
- who owns its lifecycle
- what state is its deployment in
- which host should receive the next action

Those are different categories of problem.

If Fireline blurred them together, the registry would stop being a catalog and start turning into a product-owned orchestration surface. That would make simple name resolution depend on mutable runtime state and would create a second control plane beside ACP and durable streams.

Fireline is explicitly not doing that.

## Agent Identity Is Not Deployment Identity

This is the most important design line in the registry story.

An agent catalog names agent identities.

A deployment is a running instance of some spec on some host, under some operational policy.

Those are not the same thing.

`pi-acp` is an agent identity.

"The production reviewer running in us-west with always-on wake enabled" is a deployment.

If Fireline tried to use the registry for both, the catalog would become muddy immediately:

- read-mostly metadata would mix with mutable deployment state
- agent reuse would get conflated with environment-specific rollout
- users would not know whether a name meant "what to install" or "what is currently running"

That is why the hosted-deploy work rejects "spec via agent catalog" as the deploy surface. The registry names reusable agent identities; deployment belongs elsewhere.

## Why The Fallback Must Stay Narrow

The nicest user-facing effect of the registry is small but powerful:

```ts
agent(['pi-acp'])
```

That shorthand is worth having because it makes specs portable and readable.

But the fallback must stay narrow.

Fireline should only treat unresolved single-token agent commands as registry shorthand. It should not reinterpret:

- explicit multi-token commands
- local script paths
- already-installed binaries
- arbitrary shell commands

That restraint matters because Fireline should never make command resolution feel nondeterministic. The registry is there to remove boilerplate, not to make execution ambiguous.

## Why This Composes With The Rest Of Fireline

The registry layer stays clean because it sits at the edge of the system.

It composes with:

- canonical identifiers, because runtime workflow identity still comes from ACP once the agent is running
- durable subscribers and durable promises, because those workflows begin after agent resolution, not inside the registry
- local-to-cloud handoff, because the same logical agent identity can remain readable across environments even when deployment mechanics differ

That is the right role for a registry: make agent identity portable without trying to own the rest of the architecture.

## What The User Gets

For a user, the value is concrete:

- cleaner specs
- less per-machine install glue
- a real path from a shared agent name to a usable local binary
- easier demo and onboarding stories
- no need to learn a Fireline-specific catalog format if ACP already publishes one

That is a meaningful improvement, but it stays intentionally modest. Fireline is improving the naming and install story, not promising a universal discovery plane.

## When To Use The Registry

Use the registry when the question is:

- what agent does this shared name refer to
- how can I install it locally
- how can a compose spec stay readable and portable

Do not use the registry when the question is:

- where is this deployment running
- what host owns this runtime
- how should rollout or wake policies work
- how should durable workflow state be correlated

Those belong to different layers.

## Relationship To Hosted Deploy

The registry RFC and the hosted-deploy RFC are intentionally adjacent because they are easy to confuse.

The registry owns reusable agent identity.

Hosted deploy owns deployment materialization and lifecycle.

Keeping those separate is what lets Fireline have a shared naming story without quietly turning the catalog into a deployment database.

## References

- [Proposal: ACP Registry Client Execution Plan](../../proposals/acp-registry-execution.md)
- [RFC: ACP Canonical Identifiers](./canonical-identifiers.md)
- [RFC: Durable Subscribers](./durable-subscriber.md)
- [RFC: Observability](./observability.md)
- [Proposal: Hosted Deploy Surface Decision](../../proposals/hosted-deploy-surface-decision.md)
