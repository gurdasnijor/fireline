# Sessions and ACP

A Fireline session is first an ACP session.

That matters because Fireline does not get to invent its own conversation model and then retrofit ACP on top. The protocol already defines the core shape:

- `session/new` creates a session
- `session/prompt` runs a turn inside that session
- `session/update` streams progress while the turn is running
- `session/load` resumes an existing session when the agent supports it

Fireline adds durability, approvals, projections, and host orchestration around that lifecycle. It should not replace the lifecycle itself.

## What A Session Is

An ACP session is an independent conversation context with its own history and state.

It is not:

- a sandbox id
- a WebSocket connection
- a single prompt
- a Fireline-specific database row

It is the conversation handle the agent gives back from `session/new` and expects on every later call.

That is why `SessionId` is the only canonical session identity in Fireline's agent plane. If a host dies and another host resumes the same conversation with `session/load`, the session is still the same ACP session because the `SessionId` is the same.

## The Three Identifiers To Remember

Most of the model reduces to three ACP identifiers:

- `SessionId`
  The conversation identity. Returned by `session/new`, used by `session/prompt`, `session/update`, `session/load`, and cancellation.
- `RequestId`
  The JSON-RPC request id for a specific request. For Fireline, the important cases are the `session/prompt` call itself and the `request_permission` call that can pause a tool invocation. A prompt turn is canonically identified by `(SessionId, RequestId)`, not by a Fireline-minted `prompt_turn_id`.
- `ToolCallId`
  The identity of a tool invocation within a session. If the model needs to point at one specific tool call, this is the ACP identifier to use.

The practical rule is:

- session-level identity: `SessionId`
- prompt-level identity: `(SessionId, RequestId)`
- tool-level identity: `(SessionId, ToolCallId)`

ACP libraries may serialize these as strings or JSON-RPC scalars at the wire boundary. You should still treat them as ACP-defined identifier types, not as anonymous strings you are free to reshape or hash.

## The Lifecycle

This is the session shape to keep in your head:

1. The client connects and initializes ACP.
2. The client calls `session/new`.
3. The agent returns a `SessionId`.
4. The client calls `session/prompt` with that `SessionId`.
5. While the prompt is running, the agent emits `session/update` notifications for chunks, tool activity, plans, and status changes.
6. If a tool needs approval, the agent enters the permission path and progress pauses until the decision arrives.
7. The prompt finishes with a final response and `stopReason`.
8. The same session can accept another prompt later, or a later connection can call `session/load` to resume it.

The important implication is that a session usually outlives one prompt and may outlive one connection.

## A Small Shape To Remember

```ts
const acp = await handle.connect('approval-workflow')

const { sessionId } = await acp.newSession({
  cwd: '/workspace',
  mcpServers: [],
})

await acp.prompt({
  sessionId,
  prompt: [{ type: 'text', text: 'Delete dist/, but wait for approval first.' }],
})

// Your approval surface later receives the canonical requestId for this pause.
await handle.resolvePermission(sessionId, requestId, {
  allow: true,
  resolvedBy: 'approval-workflow',
})

await acp.close()
```

What this shows:

- `newSession()` creates the conversation and returns `sessionId`
- `prompt()` starts one turn inside that conversation
- approval resolution uses the same canonical session and request identity
- `close()` closes the client connection, not the session's durable identity

If the downstream agent supports `loadSession`, a later connection can reattach:

```ts
await acp2.loadSession({
  sessionId,
  cwd: '/workspace',
  mcpServers: [],
})
```

That is the bridge between ACP's session model and Fireline's crash-proof story.

## What `session/update` Means

`session/prompt` is not a black-box RPC that stays silent until the end.

ACP defines `session/update` as the live stream of session progress. That is where clients see the turn unfold:

- model output chunks
- tool-call announcements and updates
- execution-plan updates
- other incremental session state

This is why Fireline can observe an agent live without inventing a second protocol. ACP already has a progress channel; Fireline durably records and projects it.

## Where Tool Calls And Approval Fit

Tool calls are part of the normal ACP prompt lifecycle, not a side channel.

During a prompt:

- the agent may announce a tool call
- that tool call carries a `ToolCallId`
- the tool call progresses through statuses such as `pending`, `in_progress`, `completed`, or `failed`

ACP explicitly allows `pending` to mean "awaiting approval." Fireline uses that seam to make approval durable.

That means the approval gate is not a separate conversation stitched on later. It is a durable pause inside the same ACP prompt/request lifecycle. The human decision eventually lets the same logical prompt continue or terminate.

## Why Canonical ACP Identifiers Matter In Fireline

Fireline's canonical-id work is really a promise not to smuggle in a second identity system.

The bad version of this architecture invents:

- a prompt-turn id
- a hash-based approval request id
- a bespoke lineage edge table

The ACP-shaped version does not need those.

It can say:

- the session is `SessionId`
- the prompt request is `(SessionId, RequestId)`
- the tool invocation is `(SessionId, ToolCallId)`
- lineage rides through ACP `_meta` and trace context, not a Fireline-only graph id

That gives you a cleaner mental model and a safer system. A dashboard, a subscriber driver, and a resumed host can all talk about the same work item without translating through Fireline-local surrogate ids first.

## `RequestId` Is Not A UUID Contract

This is the identifier people misuse most often.

`RequestId` is the JSON-RPC request id. It is not "the prompt UUID." Depending on the client and agent surface, it may be a string, number, or `null` at the protocol layer.

So the rule is:

- preserve it exactly
- treat it as ACP identity
- do not hash it, reformat it, or replace it with a Fireline-generated id

In Fireline terms, a prompt turn is "the `session/prompt` request with this exact `RequestId` inside this exact `SessionId`." An approval pause is the same discipline applied to the permission request: preserve the ACP request id instead of minting a Fireline-local surrogate.

## Gotchas

- Do not confuse session identity with connection identity.
  `acp.close()` ends the transport session, not necessarily the ACP session.
- Do not confuse one prompt with one session.
  A session usually spans multiple prompts.
- Do not invent a Fireline prompt id.
  Prompt identity is `(SessionId, RequestId)`.
- Do not treat tool calls as free-floating events.
  They belong to one session and one ACP prompt lifecycle, and `ToolCallId` is the canonical handle when you need one specific invocation.
- Do not assume approval is outside ACP.
  It is a pause in the same prompt flow, made durable by Fireline.

## Read This Next

- [Durable Streams](./durable-streams.md)
- [Approvals](../approvals.md)
- [Compose and Start](../compose-and-start.md)
- [docs/proposals/acp-canonical-identifiers.md](../../proposals/acp-canonical-identifiers.md)
- [ACP Schema](https://agentclientprotocol.com/protocol/schema)
