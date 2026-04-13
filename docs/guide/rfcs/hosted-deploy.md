# RFC: Hosted Deploy

> Status: design rationale
> Audience: engineers deciding whether hosted Fireline deployment should be its own platform protocol or should compose from OCI artifacts, durable streams, and subscriber profiles

Fireline should not grow a deployment control plane just because hosted deployment matters.

The hosted deploy story is stronger if it reuses the substrates Fireline already believes in:

- OCI artifacts for packaging
- target-native deployment tooling for execution
- durable streams for durable infrastructure inputs when multi-spec materialization is needed
- durable-subscriber profiles for wake and deployment reconciliation

That is the design.

## The Problem Fireline Is Avoiding

Hosted deployment is where systems are tempted to add "just one more API."

There is already a spec. There is already a host. There is already an operator action called deploy. The obvious move is to add a Fireline-owned HTTP endpoint that accepts the spec and manages deployment state.

That looks tidy for one phase. Then it starts to sprawl:

- deploy becomes a second control plane beside ACP and durable streams
- CLI verbs quietly become wrappers around a Fireline-specific protocol
- deployment state starts competing with the agent-plane and infrastructure-plane models
- every future hosting feature has to decide which surface is truly authoritative

Fireline is choosing not to pay that debt.

## The Decision

Hosted deployment is tiered, but both tiers reuse existing substrates instead of inventing a new protocol.

Tier A:

- the spec is embedded into an OCI artifact at build time
- deployment uses target-native tooling such as `fly deploy`, `kubectl apply`, or equivalent platform commands
- the host boots, reads the embedded spec, and starts serving

Tier C:

- the spec is appended to a durable-streams resource
- a `DeploymentSpecSubscriber` materializes it
- the host reconciles that durable input without a new Fireline deploy API

Both tiers keep the same architectural promise: deployment is expressed through packaging and durable inputs, not through a Fireline-owned HTTP control surface.

## Why OCI Is The Right Primary Deploy Story

OCI packaging maps cleanly to how developers already think about deploying services.

You build an artifact once.
You publish it somewhere standard.
You hand it to the platform that already knows how to run containers.

That buys Fireline three important things:

- one artifact story across local, CI, and hosted environments
- a deployment UX that uses platform-native verbs instead of Fireline-specific ones
- a cleaner separation between "what is the app?" and "where is it running?"

This is why the primary deploy path is not "send a spec to Fireline." It is "build the image that contains the spec and let the target platform do what target platforms already do."

## Why The Durable-Streams Path Exists

Tier C exists because one image per spec is not the whole hosted story.

At some point, hosted Fireline needs a durable, replayable input for deployment intent:

- multiple specs
- live updates
- tenant-scoped deployment resources
- replay-safe materialization after restart

Durable streams are already the substrate Fireline trusts for append-only, replayable truth. So when hosted deploy grows beyond the embedded-spec path, the natural extension is not a deploy API. It is a durable deployment-spec resource consumed by a subscriber profile.

That keeps the later path aligned with the earlier path:

- Tier A uses an artifact as the deployment input
- Tier C uses a durable stream as the deployment input

Neither requires inventing a third protocol.

## Why Deployment Is Not Agent Identity

Hosted deploy becomes confused quickly if deployment identity and agent identity get mixed together.

An agent identity answers:

- what reusable agent is this
- what should a shared name like `pi-acp` refer to

A deployment answers:

- which concrete hosted instance should exist
- under what runtime and wake policy
- on what platform and in what operational context

Those are different questions.

That is why the registry/catalog layer should not be reused as the deploy surface. The catalog names reusable agents. Hosted deploy materializes running instances. Mixing those together would make the naming layer carry mutable operational meaning it was never designed to own.

## Why `alwaysOn` Belongs In Spec Metadata

Always-on behavior is real, but it is not a separate deployment protocol.

It is deployment intent expressed as spec metadata and enforced by the right substrate for that tier:

- platform keepalive and replica settings in the Tier A image-driven path
- `AlwaysOnDeploymentSubscriber` in the Tier C durable-stream path

This is a good example of the general rule.

The deploy story stays clean when Fireline treats operational policy as part of the spec and lets existing mechanisms consume it. It gets messy when the CLI or a control-plane endpoint invents a second lifecycle contract around the same concept.

## Why This Keeps Control-Plane Count Low

Fireline already has enough real boundaries:

- ACP for runtime interaction
- durable streams for durable state and durable infrastructure inputs
- provider and sandbox abstractions for execution substrate

Adding a Fireline deployment API would not simplify that architecture. It would make the architecture explain the same intent through one more surface.

The current design is better because it keeps each boundary honest:

- package and deploy with container-native tooling
- append durable deployment intent through durable streams when needed
- materialize and wake deployments through subscriber profiles

That gives Fireline hosted deployment without making Fireline itself into a deployment protocol.

## What The User Gets

For a user, this produces a more legible story:

- build once
- run locally or ship the same artifact to a real platform
- use target-native deployment flows where possible
- use durable-stream append when the hosted model genuinely needs stream-driven materialization

That is easier to trust than a product-specific deploy endpoint because it keeps the verbs honest.

`build` means build.
`deploy` means hand the artifact to the platform.
`push` means append to a stream.

The architecture should read that clearly.

## Why This Composes With The Rest Of Fireline

Hosted deploy is not a special world outside the rest of the product. It composes with the same architectural layers:

- canonical identifiers still govern the agent plane once the deployment is running
- durable subscribers still own wake and deployment materialization behavior
- observability still follows trace propagation instead of a bespoke deployment graph
- the registry still names reusable agents without becoming a deployment database

That coherence is the actual benefit of the design. Hosted deploy feels smaller because it is made out of things Fireline already trusts.

## When To Reach For Which Tier

Reach for Tier A when:

- one image per spec is acceptable
- target-native deployment is the right operator story
- you want the simplest path from local build to hosted runtime

Reach for Tier C when:

- deployment intent needs to be durable and replayable as stream input
- multiple specs or live updates matter
- subscriber-driven materialization is worth the extra machinery

What should not change between tiers is the architectural rule: no new Fireline deployment protocol.

## Relationship To Registry And Observability

Hosted deploy stays clean because two nearby layers keep their own jobs:

- the registry names agent identities; it does not name running deployments
- observability explains hosted execution through traces and durable state; it does not require a bespoke deployment graph

That separation is what keeps hosted deploy from turning into a dumping ground for unrelated concerns.

## References

- [Proposal: Hosted Fireline Deployment](../../proposals/hosted-fireline-deployment.md)
- [Proposal: Hosted Deploy Surface Decision](../../proposals/hosted-deploy-surface-decision.md)
- [Proposal: Fireline CLI Production-Readiness Gap Analysis + Design](../../proposals/fireline-cli-execution.md)
- [RFC: ACP Registry](./registry.md)
- [RFC: Observability](./observability.md)
- [RFC: Durable Subscribers](./durable-subscriber.md)
