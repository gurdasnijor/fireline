# Handoff Dry Run: Local -> Docker Session Handoff (2026-04-13)

Status: `FAIL`

Scope run:
- `fireline build docs/demos/assets/agent.ts --target docker`
- local host on `:4440` against shared durable-streams on `:7474`
- same session id resumed against docker host on `:4441`

Driver:
- [scripts/demo-handoff.sh](/Users/gnijor/gurdasnijor/fireline/scripts/demo-handoff.sh)

## Verdict

The final demo act is still red on current `main`.

What worked:
- `fireline build ... --target docker` completed and emitted `fireline-agent:latest`
- local host boot on `:4440` was healthy and reused the existing `fireline-streams` daemon on `:7474`
- a real session was created on the local host and two turns completed on the shared state stream after resolving the prompt-level approval fallback
- the docker host image can reach `host.docker.internal:7474` from inside the container

What failed:
- the built image still booted the placeholder embedded spec, not `docs/demos/assets/agent.ts`
- when the real demo spec was mounted into the image, embedded-spec bootstrap failed with `embedded-spec boot does not support resource mounts`
- when a docker-safe override spec was mounted into the same built image, `fireline repl <session-id>` against `:4441` failed with `Resource not found: <session-id>`
- the lower-level ACP check matched the CLI failure: `loadSession(...)` on the docker host returned `RequestError: Resource not found`

## Evidence

Local session:
- state stream: `mono-80f-handoff-1776065049`
- session id: `f155bf26-c135-4279-8a3f-58886f1646c9`
- first turn response: `stored cobalt-kite`
- second turn response: `will-remember`

Built image bug:
- build log copied `docker/specs/placeholder-spec.json` into `/etc/fireline/spec.json`
- container log: `fireline: booting embedded spec 'placeholder' from /etc/fireline/spec.json via existing compose()->start lowering`

Real demo spec mount bug:
- container log: `fireline: embedded-spec bootstrap failed: embedded-spec boot does not support resource mounts`

Docker-safe override handoff failure:
- standalone CLI attach: `FIRELINE_URL=http://127.0.0.1:4441 fireline repl f155bf26-c135-4279-8a3f-58886f1646c9`
- observed stderr: `fireline: Resource not found: f155bf26-c135-4279-8a3f-58886f1646c9`
- direct ACP probe matched it: `loadSession(...)` returned `RequestError: Resource not found`

## Rough Timings

Approximate dry-run timings on this machine:
- `fireline build --target docker`: ~4m52s
- local host ready on `:4440`: <1s after launch, reusing existing streams on `:7474`
- first local turn: approval waited externally, then completed in a few seconds after resolution
- second local turn: a few seconds after approval resolution
- docker host boot with placeholder or override spec: <5s to healthz
- docker attach failure: immediate once `fireline repl <session-id>` hit `session/load`

## Surprises

- The local demo spec still triggers the prompt-level approval fallback even for plain text prompts. That means the handoff driver has to resolve approval rows before any turn completes.
- The docker image bug and the resource-mount limitation are distinct blockers. Fixing the build arg mismatch alone is not enough for `docs/demos/assets/agent.ts`.
- The docker-safe override proved the container could reach the shared durable-streams host, so the final `Resource not found` failure is not just a networking issue.

## Follow-up Bugs Filed

- `mono-80f.3` build path embeds placeholder spec instead of the requested Fireline spec
- `mono-80f.1` embedded-spec docker bootstrap rejects Fireline resource mounts
- `mono-80f.2` local `fireline run` session cannot be resumed from the docker host via `fireline repl <session-id>`

## Operator Read

This is not ready for the final live handoff beat yet.

The closest truthful statement today is:
- local session state is durable on the shared stream
- the docker host can boot and see the stream
- but the advertised local->docker `session/load` handoff still fails before the resumed prompt can continue
