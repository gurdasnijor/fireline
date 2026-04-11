---- MODULE managed_agents ----
EXTENDS Naturals, Sequences, FiniteSets, TLC

\* Compact abstract model for Fireline's managed-agent substrate.
\*
\* Scope:
\* - Session as an append-only event log with offset replay and producer-tuple dedupe
\* - Orchestration as wake-by-composition over Session + Host state
\* - Harness as "every visible effect lands in Session"
\* - Host as provision/reuse/stop/reprovision of reachable runtimes
\* - Sandbox as provision/execute/stop of isolated tool-call executors
\* - Resources as requested {source_ref, mount_path} pairs copied into a runtime
\* - Tools as schema-only projections of portable capability attachments
\*
\* Non-goals:
\* - ACP wire transport
\* - concrete provider behavior (Docker, local process, etc.)
\* - shell internals
\* - full durable-streams protocol

CONSTANTS
  Sessions,
  RuntimeKeys,
  RuntimeIds,
  SandboxIds,
  RequestIds,
  ToolNames,
  Sources,
  MountPaths,
  LogicalEventIds,
  ProducerIds,
  ProducerEpochs,
  ProducerSeqs,
  Callers

EventKinds ==
  {
    "session_created",
    "prompt_turn_started",
    "chunk_appended",
    "permission_requested",
    "approval_resolved",
    "tool_descriptor_emitted",
    "fs_op_captured",
    "runtime_stopped",
    "runtime_woken",
    "session_loaded"
  }

DefaultSession == CHOOSE s \in Sessions : TRUE
DefaultRuntimeKey == CHOOSE rk \in RuntimeKeys : TRUE
DefaultRuntimeId == CHOOSE rid \in RuntimeIds : TRUE
DefaultProducerEpoch == CHOOSE epoch \in ProducerEpochs : TRUE
NoRequest == "no_request"
NoRuntimeId == "no_runtime"

InitialSessionLog ==
  [s \in Sessions |-> <<>>]

InitialRuntimeIndex ==
  [ rk \in RuntimeKeys |->
      [ status |-> "stopped",
        runtimeId |-> DefaultRuntimeId,
        specPresent |-> FALSE,
        boundSessions |-> {}
      ]
  ]

InitialSandboxIndex ==
  [ sb \in SandboxIds |->
      [ status |-> "stopped",
        runtimeKey |-> DefaultRuntimeKey
      ]
  ]

ToolDescriptor(name) ==
  [ name |-> name,
    description |-> "opaque description",
    inputSchema |-> "opaque schema"
  ]

CapabilityRef(name) ==
  [ descriptor |-> ToolDescriptor(name),
    transportRef |-> "opaque_transport",
    credentialRef |-> "opaque_credential"
  ]

ResourcePair(source, mount) ==
  [ sourceRef |-> source, mountPath |-> mount ]

IsPrefix(prefix, full) ==
  /\ Len(prefix) <= Len(full)
  /\ SubSeq(full, 1, Len(prefix)) = prefix

ReplaySuffix(log, offset) ==
  IF offset = 0 THEN log
  ELSE IF offset > Len(log) THEN <<>>
  ELSE SubSeq(log, offset + 1, Len(log))

CommitTuple(producerId, epoch, seq) ==
  [ producerId |-> producerId,
    epoch |-> epoch,
    seq |-> seq
  ]

Occurrences(log, producerId, epoch, seq) ==
  Cardinality(
    { i \in 1..Len(log) :
        /\ log[i].producerId = producerId
        /\ log[i].epoch = epoch
        /\ log[i].seq = seq
    }
  )

FirstMatchingResolution(history) ==
  IF Len(history) = 0 THEN "none" ELSE history[1]

RecordedTools(history) ==
  { history[i] : i \in 1..Len(history) }

NextWakeEpoch(epoch) ==
  IF \E next \in ProducerEpochs : next # epoch
  THEN CHOOSE next \in ProducerEpochs : next # epoch
  ELSE epoch

VARIABLES
  sessionLog,
  runtimeIndex,
  sandboxIndex,
  pendingApprovals,
  toolRegistry,
  capabilityRefs,
  requestedResources,
  mountedResources,
  sandboxToolHistory,
  seenCommits,
  approvalHistory,
  visibleEffects,
  blockedRequests,
  releasedRequests,
  reachable,
  lastReplay,
  stopSnapshot,
  wakeEpoch,
  responseEpochs,
  lastWake,
  wakeResponses,
  previousSessionLog,
  previousRuntimeIndex,
  lastAction

Vars ==
  << sessionLog,
     runtimeIndex,
     sandboxIndex,
     pendingApprovals,
     toolRegistry,
     capabilityRefs,
     requestedResources,
     mountedResources,
     sandboxToolHistory,
     seenCommits,
     approvalHistory,
     visibleEffects,
     blockedRequests,
     releasedRequests,
     reachable,
     lastReplay,
     stopSnapshot,
     wakeEpoch,
     responseEpochs,
     lastWake,
     wakeResponses,
     previousSessionLog,
     previousRuntimeIndex,
     lastAction >>

Init ==
  /\ sessionLog = InitialSessionLog
  /\ runtimeIndex = InitialRuntimeIndex
  /\ sandboxIndex = InitialSandboxIndex
  /\ pendingApprovals =
      [ req \in RequestIds |->
          [ sessionId |-> DefaultSession,
            state |-> "none"
          ]
      ]
  /\ toolRegistry = [t \in ToolNames |-> ToolDescriptor(t)]
  /\ capabilityRefs = [t \in ToolNames |-> CapabilityRef(t)]
  /\ requestedResources = [rk \in RuntimeKeys |-> {}]
  /\ mountedResources = [rid \in RuntimeIds |-> {}]
  /\ sandboxToolHistory = [sb \in SandboxIds |-> <<>>]
  /\ seenCommits = [s \in Sessions |-> {}]
  /\ approvalHistory = [req \in RequestIds |-> <<>>]
  /\ visibleEffects = [s \in Sessions |-> <<>>]
  /\ blockedRequests = [s \in Sessions |-> NoRequest]
  /\ releasedRequests = {}
  /\ reachable = [rk \in RuntimeKeys |-> FALSE]
  /\ lastReplay =
      [s \in Sessions |->
        [ offset |-> 0,
          capturedLog |-> <<>>,
          suffix |-> <<>>
        ]
      ]
  /\ stopSnapshot = [rk \in RuntimeKeys |-> [s \in Sessions |-> <<>>]]
  /\ wakeEpoch = DefaultProducerEpoch
  /\ responseEpochs = [c \in Callers |-> DefaultProducerEpoch]
  /\ lastWake =
      [ valid |-> FALSE,
        caller |-> CHOOSE c \in Callers : TRUE,
        sessionId |-> DefaultSession,
        runtimeKey |-> DefaultRuntimeKey,
        beforeStatus |-> "stopped",
        beforeRuntimeId |-> DefaultRuntimeId,
        afterRuntimeId |-> DefaultRuntimeId,
        createdNew |-> FALSE
      ]
  /\ wakeResponses = [c \in Callers |-> NoRuntimeId]
  /\ previousSessionLog = InitialSessionLog
  /\ previousRuntimeIndex = InitialRuntimeIndex
  /\ lastAction = "init"

RecordStep(actionName) ==
  /\ previousSessionLog' = sessionLog
  /\ previousRuntimeIndex' = runtimeIndex
  /\ lastAction' = actionName

ProvisionRuntime(rk, rid, s, source, mount) ==
  /\ runtimeIndex[rk].status = "stopped"
  /\ runtimeIndex' =
      [runtimeIndex EXCEPT ![rk] =
        [ status |-> "ready",
          runtimeId |-> rid,
          specPresent |-> TRUE,
          boundSessions |-> {s}
        ]
      ]
  /\ requestedResources' = [requestedResources EXCEPT ![rk] = {ResourcePair(source, mount)}]
  /\ mountedResources' = [mountedResources EXCEPT ![rid] = {ResourcePair(source, mount)}]
  /\ reachable' = [reachable EXCEPT ![rk] = TRUE]
  /\ RecordStep("provision_runtime")
  /\ UNCHANGED
      << sessionLog,
         sandboxIndex,
         pendingApprovals,
         toolRegistry,
         capabilityRefs,
         sandboxToolHistory,
         seenCommits,
         approvalHistory,
         visibleEffects,
         blockedRequests,
         releasedRequests,
         lastReplay,
         stopSnapshot,
         wakeEpoch,
         responseEpochs,
         lastWake,
         wakeResponses >>

HarnessEmit(s, rk, logicalId, kind, producerId, epoch, seq) ==
  /\ runtimeIndex[rk].status = "ready"
  /\ s \in runtimeIndex[rk].boundSessions
  /\ CommitTuple(producerId, epoch, seq) \notin seenCommits[s]
  /\ kind \in {"session_created", "prompt_turn_started", "chunk_appended", "fs_op_captured"}
  /\ sessionLog' =
      [sessionLog EXCEPT ![s] =
        Append(
          @,
          [ logicalId |-> logicalId,
            producerId |-> producerId,
            epoch |-> epoch,
            seq |-> seq,
            kind |-> kind
          ]
        )
      ]
  /\ visibleEffects' =
      [visibleEffects EXCEPT ![s] = Append(@, [logicalId |-> logicalId, kind |-> kind])]
  /\ seenCommits' = [seenCommits EXCEPT ![s] = @ \cup {CommitTuple(producerId, epoch, seq)}]
  /\ RecordStep("harness_emit")
  /\ UNCHANGED
      << runtimeIndex,
         sandboxIndex,
         pendingApprovals,
         toolRegistry,
         capabilityRefs,
         requestedResources,
         mountedResources,
         sandboxToolHistory,
         approvalHistory,
         blockedRequests,
         releasedRequests,
         reachable,
         lastReplay,
         stopSnapshot,
         wakeEpoch,
         responseEpochs,
         lastWake,
         wakeResponses >>

RetryAppend(s, producerId, epoch, seq) ==
  /\ CommitTuple(producerId, epoch, seq) \in seenCommits[s]
  /\ RecordStep("retry_append")
  /\ UNCHANGED
      << sessionLog,
         runtimeIndex,
         sandboxIndex,
         pendingApprovals,
         toolRegistry,
         capabilityRefs,
         requestedResources,
         mountedResources,
         sandboxToolHistory,
         seenCommits,
         approvalHistory,
         visibleEffects,
         blockedRequests,
         releasedRequests,
         reachable,
         lastReplay,
         stopSnapshot,
         wakeEpoch,
         responseEpochs,
         lastWake,
         wakeResponses >>

ReplayFromOffset(s, offset) ==
  /\ offset \in 0..Len(sessionLog[s])
  /\ lastReplay' =
      [lastReplay EXCEPT ![s] =
        [ offset |-> offset,
          capturedLog |-> sessionLog[s],
          suffix |-> ReplaySuffix(sessionLog[s], offset)
        ]
      ]
  /\ RecordStep("replay_from_offset")
  /\ UNCHANGED
      << sessionLog,
         runtimeIndex,
         sandboxIndex,
         pendingApprovals,
         toolRegistry,
         capabilityRefs,
         requestedResources,
         mountedResources,
         sandboxToolHistory,
         seenCommits,
         approvalHistory,
         visibleEffects,
         blockedRequests,
         releasedRequests,
         reachable,
         stopSnapshot,
         wakeEpoch,
         responseEpochs,
         lastWake,
         wakeResponses >>

RequestApproval(s, req, logicalId, producerId, epoch, seq) ==
  /\ blockedRequests[s] = NoRequest
  /\ CommitTuple(producerId, epoch, seq) \notin seenCommits[s]
  /\ sessionLog' =
      [sessionLog EXCEPT ![s] =
        Append(
          @,
          [ logicalId |-> logicalId,
            producerId |-> producerId,
            epoch |-> epoch,
            seq |-> seq,
            kind |-> "permission_requested"
          ]
        )
      ]
  /\ pendingApprovals' =
      [pendingApprovals EXCEPT ![req] = [sessionId |-> s, state |-> "pending"]]
  /\ blockedRequests' = [blockedRequests EXCEPT ![s] = req]
  /\ seenCommits' = [seenCommits EXCEPT ![s] = @ \cup {CommitTuple(producerId, epoch, seq)}]
  /\ RecordStep("request_approval")
  /\ UNCHANGED
      << runtimeIndex,
         sandboxIndex,
         toolRegistry,
         capabilityRefs,
         requestedResources,
         mountedResources,
         sandboxToolHistory,
         approvalHistory,
         visibleEffects,
         releasedRequests,
         reachable,
         lastReplay,
         stopSnapshot,
         wakeEpoch,
         responseEpochs,
         lastWake,
         wakeResponses >>

ResolveApproval(req, logicalId, producerId, epoch, seq, allow) ==
  LET s == pendingApprovals[req].sessionId IN
  /\ pendingApprovals[req].state \in {"pending", "resolved_allow", "resolved_deny"}
  /\ CommitTuple(producerId, epoch, seq) \notin seenCommits[s]
  /\ sessionLog' =
      [sessionLog EXCEPT ![s] =
        Append(
          @,
          [ logicalId |-> logicalId,
            producerId |-> producerId,
            epoch |-> epoch,
            seq |-> seq,
            kind |-> "approval_resolved"
          ]
        )
      ]
  /\ pendingApprovals' =
      [pendingApprovals EXCEPT ![req].state =
        IF allow THEN "resolved_allow" ELSE "resolved_deny"
      ]
  /\ approvalHistory' =
      [approvalHistory EXCEPT ![req] = Append(@, IF allow THEN "allow" ELSE "deny")]
  /\ seenCommits' = [seenCommits EXCEPT ![s] = @ \cup {CommitTuple(producerId, epoch, seq)}]
  /\ RecordStep("resolve_approval")
  /\ UNCHANGED
      << runtimeIndex,
         sandboxIndex,
         toolRegistry,
         capabilityRefs,
         requestedResources,
         mountedResources,
         sandboxToolHistory,
         visibleEffects,
         blockedRequests,
         releasedRequests,
         reachable,
         lastReplay,
         stopSnapshot,
         wakeEpoch,
         responseEpochs,
         lastWake,
         wakeResponses >>

AdvanceBlockedRequest(s, req) ==
  /\ blockedRequests[s] = req
  /\ FirstMatchingResolution(approvalHistory[req]) = "allow"
  /\ blockedRequests' = [blockedRequests EXCEPT ![s] = NoRequest]
  /\ releasedRequests' = releasedRequests \cup {req}
  /\ RecordStep("advance_blocked_request")
  /\ UNCHANGED
      << sessionLog,
         runtimeIndex,
         sandboxIndex,
         pendingApprovals,
         toolRegistry,
         capabilityRefs,
         requestedResources,
         mountedResources,
         sandboxToolHistory,
         seenCommits,
         approvalHistory,
         visibleEffects,
         reachable,
         lastReplay,
         stopSnapshot,
         wakeEpoch,
         responseEpochs,
         lastWake,
         wakeResponses >>

StopRuntime(rk) ==
  /\ runtimeIndex[rk].status = "ready"
  /\ runtimeIndex' = [runtimeIndex EXCEPT ![rk].status = "stopped"]
  /\ reachable' = [reachable EXCEPT ![rk] = FALSE]
  /\ stopSnapshot' = [stopSnapshot EXCEPT ![rk] = sessionLog]
  /\ RecordStep("stop_runtime")
  /\ UNCHANGED
      << sessionLog,
         sandboxIndex,
         pendingApprovals,
         toolRegistry,
         capabilityRefs,
         requestedResources,
         mountedResources,
         sandboxToolHistory,
         seenCommits,
         approvalHistory,
         visibleEffects,
         blockedRequests,
         releasedRequests,
         lastReplay,
         wakeEpoch,
         responseEpochs,
         lastWake,
         wakeResponses >>

SandboxProvision(sb, rk) ==
  /\ sandboxIndex[sb].status = "stopped"
  /\ runtimeIndex[rk].status = "ready"
  /\ reachable[rk]
  /\ sandboxIndex' =
      [sandboxIndex EXCEPT ![sb] =
        [ status |-> "ready",
          runtimeKey |-> rk
        ]
      ]
  /\ sandboxToolHistory' = [sandboxToolHistory EXCEPT ![sb] = <<>>]
  /\ RecordStep("sandbox_provision")
  /\ UNCHANGED
      << sessionLog,
         runtimeIndex,
         pendingApprovals,
         toolRegistry,
         capabilityRefs,
         requestedResources,
         mountedResources,
         seenCommits,
         approvalHistory,
         visibleEffects,
         blockedRequests,
         releasedRequests,
         reachable,
         lastReplay,
         stopSnapshot,
         wakeEpoch,
         responseEpochs,
         lastWake,
         wakeResponses >>

SandboxExecute(sb, tool) ==
  /\ sandboxIndex[sb].status = "ready"
  /\ runtimeIndex[sandboxIndex[sb].runtimeKey].status = "ready"
  /\ tool \in ToolNames
  /\ tool \notin RecordedTools(sandboxToolHistory[sb])
  /\ sandboxToolHistory' = [sandboxToolHistory EXCEPT ![sb] = Append(@, tool)]
  /\ RecordStep("sandbox_execute")
  /\ UNCHANGED
      << sessionLog,
         runtimeIndex,
         sandboxIndex,
         pendingApprovals,
         toolRegistry,
         capabilityRefs,
         requestedResources,
         mountedResources,
         seenCommits,
         approvalHistory,
         visibleEffects,
         blockedRequests,
         releasedRequests,
         reachable,
         lastReplay,
         stopSnapshot,
         wakeEpoch,
         responseEpochs,
         lastWake,
         wakeResponses >>

SandboxStop(sb) ==
  /\ sandboxIndex[sb].status = "ready"
  /\ sandboxIndex' =
      [sandboxIndex EXCEPT ![sb] =
        [ status |-> "stopped",
          runtimeKey |-> DefaultRuntimeKey
        ]
      ]
  /\ sandboxToolHistory' = [sandboxToolHistory EXCEPT ![sb] = <<>>]
  /\ RecordStep("sandbox_stop")
  /\ UNCHANGED
      << sessionLog,
         runtimeIndex,
         pendingApprovals,
         toolRegistry,
         capabilityRefs,
         requestedResources,
         mountedResources,
         seenCommits,
         approvalHistory,
         visibleEffects,
         blockedRequests,
         releasedRequests,
         reachable,
         lastReplay,
         stopSnapshot,
         wakeEpoch,
         responseEpochs,
         lastWake,
         wakeResponses >>

WakeReady(caller, s, rk) ==
  LET sameEpisode ==
        /\ lastWake.valid
        /\ lastWake.sessionId = s
        /\ lastWake.runtimeKey = rk
        /\ lastWake.beforeStatus = "ready"
        /\ lastWake.afterRuntimeId = runtimeIndex[rk].runtimeId
        /\ lastWake.createdNew = FALSE
      newEpoch ==
        IF sameEpisode THEN wakeEpoch ELSE NextWakeEpoch(wakeEpoch)
  IN
  /\ runtimeIndex[rk].status = "ready"
  /\ s \in runtimeIndex[rk].boundSessions
  /\ wakeEpoch' = newEpoch
  /\ responseEpochs' =
      [ c \in Callers |->
          IF sameEpisode THEN
            IF c = caller THEN newEpoch ELSE responseEpochs[c]
          ELSE
            IF c = caller THEN newEpoch ELSE DefaultProducerEpoch
      ]
  /\ lastWake' =
      [ valid |-> TRUE,
        caller |-> caller,
        sessionId |-> s,
        runtimeKey |-> rk,
        beforeStatus |-> "ready",
        beforeRuntimeId |-> runtimeIndex[rk].runtimeId,
        afterRuntimeId |-> runtimeIndex[rk].runtimeId,
        createdNew |-> FALSE
      ]
  /\ wakeResponses' =
      [ c \in Callers |->
          IF sameEpisode THEN
            IF c = caller THEN runtimeIndex[rk].runtimeId ELSE wakeResponses[c]
          ELSE
            IF c = caller THEN runtimeIndex[rk].runtimeId ELSE NoRuntimeId
      ]
  /\ RecordStep("wake_ready")
  /\ UNCHANGED
      << sessionLog,
         runtimeIndex,
         sandboxIndex,
         pendingApprovals,
         toolRegistry,
         capabilityRefs,
         requestedResources,
         mountedResources,
         sandboxToolHistory,
         seenCommits,
         approvalHistory,
         visibleEffects,
         blockedRequests,
         releasedRequests,
         reachable,
         lastReplay,
         stopSnapshot >>

WakeStopped(caller, s, rk, newRid) ==
  LET sameEpisode ==
        /\ lastWake.valid
        /\ lastWake.sessionId = s
        /\ lastWake.runtimeKey = rk
        /\ lastWake.beforeStatus = "stopped"
        /\ lastWake.afterRuntimeId = newRid
        /\ lastWake.createdNew = TRUE
      newEpoch ==
        IF sameEpisode THEN wakeEpoch ELSE NextWakeEpoch(wakeEpoch)
  IN
  /\ runtimeIndex[rk].status = "stopped"
  /\ runtimeIndex[rk].specPresent
  /\ s \in runtimeIndex[rk].boundSessions
  /\ newRid # runtimeIndex[rk].runtimeId
  /\ wakeEpoch' = newEpoch
  /\ responseEpochs' =
      [ c \in Callers |->
          IF sameEpisode THEN
            IF c = caller THEN newEpoch ELSE responseEpochs[c]
          ELSE
            IF c = caller THEN newEpoch ELSE DefaultProducerEpoch
      ]
  /\ runtimeIndex' =
      [runtimeIndex EXCEPT ![rk] =
        [ @ EXCEPT !.status = "ready", !.runtimeId = newRid ]
      ]
  /\ mountedResources' = [mountedResources EXCEPT ![newRid] = requestedResources[rk]]
  /\ reachable' = [reachable EXCEPT ![rk] = TRUE]
  /\ lastWake' =
      [ valid |-> TRUE,
        caller |-> caller,
        sessionId |-> s,
        runtimeKey |-> rk,
        beforeStatus |-> "stopped",
        beforeRuntimeId |-> runtimeIndex[rk].runtimeId,
        afterRuntimeId |-> newRid,
        createdNew |-> TRUE
      ]
  /\ wakeResponses' =
      [ c \in Callers |->
          IF sameEpisode THEN
            IF c = caller THEN newRid ELSE wakeResponses[c]
          ELSE
            IF c = caller THEN newRid ELSE NoRuntimeId
      ]
  /\ RecordStep("wake_stopped")
  /\ UNCHANGED
      << sessionLog,
         sandboxIndex,
         pendingApprovals,
         toolRegistry,
         capabilityRefs,
         requestedResources,
         seenCommits,
         sandboxToolHistory,
         approvalHistory,
         visibleEffects,
         blockedRequests,
         releasedRequests,
         lastReplay,
         stopSnapshot >>

CurrentWakeResponses ==
  { wakeResponses[c] :
      c \in { d \in Callers :
                /\ responseEpochs[d] = wakeEpoch
                /\ wakeResponses[d] # NoRuntimeId
             }
  }

Next ==
  \/ \E rk \in RuntimeKeys, rid \in RuntimeIds, s \in Sessions, source \in Sources, mount \in MountPaths :
       ProvisionRuntime(rk, rid, s, source, mount)
  \/ \E s \in Sessions, rk \in RuntimeKeys, logicalId \in LogicalEventIds, producerId \in ProducerIds, epoch \in ProducerEpochs, seq \in ProducerSeqs :
       HarnessEmit(s, rk, logicalId, "session_created", producerId, epoch, seq)
  \/ \E s \in Sessions, rk \in RuntimeKeys, logicalId \in LogicalEventIds, producerId \in ProducerIds, epoch \in ProducerEpochs, seq \in ProducerSeqs :
       HarnessEmit(s, rk, logicalId, "prompt_turn_started", producerId, epoch, seq)
  \/ \E s \in Sessions, rk \in RuntimeKeys, logicalId \in LogicalEventIds, producerId \in ProducerIds, epoch \in ProducerEpochs, seq \in ProducerSeqs :
       HarnessEmit(s, rk, logicalId, "fs_op_captured", producerId, epoch, seq)
  \/ \E s \in Sessions, producerId \in ProducerIds, epoch \in ProducerEpochs, seq \in ProducerSeqs :
       RetryAppend(s, producerId, epoch, seq)
  \/ \E s \in Sessions :
       \E offset \in 0..Len(sessionLog[s]) :
         ReplayFromOffset(s, offset)
  \/ \E s \in Sessions, req \in RequestIds, logicalId \in LogicalEventIds, producerId \in ProducerIds, epoch \in ProducerEpochs, seq \in ProducerSeqs :
       RequestApproval(s, req, logicalId, producerId, epoch, seq)
  \/ \E req \in RequestIds, logicalId \in LogicalEventIds, producerId \in ProducerIds, epoch \in ProducerEpochs, seq \in ProducerSeqs :
       ResolveApproval(req, logicalId, producerId, epoch, seq, TRUE)
  \/ \E req \in RequestIds, logicalId \in LogicalEventIds, producerId \in ProducerIds, epoch \in ProducerEpochs, seq \in ProducerSeqs :
       ResolveApproval(req, logicalId, producerId, epoch, seq, FALSE)
  \/ \E s \in Sessions, req \in RequestIds :
       AdvanceBlockedRequest(s, req)
  \/ \E rk \in RuntimeKeys :
       StopRuntime(rk)
  \/ \E sb \in SandboxIds, rk \in RuntimeKeys :
       SandboxProvision(sb, rk)
  \/ \E sb \in SandboxIds, tool \in ToolNames :
       SandboxExecute(sb, tool)
  \/ \E sb \in SandboxIds :
       SandboxStop(sb)
  \/ \E caller \in Callers, s \in Sessions, rk \in RuntimeKeys :
       WakeReady(caller, s, rk)
  \/ \E caller \in Callers, s \in Sessions, rk \in RuntimeKeys, newRid \in RuntimeIds :
       WakeStopped(caller, s, rk, newRid)

Spec == Init /\ [][Next]_Vars

SessionAppendOnly ==
  \A s \in Sessions :
    IsPrefix(previousSessionLog[s], sessionLog[s])

SessionReplayFromOffsetIsSuffix ==
  \A s \in Sessions :
    lastReplay[s].suffix = ReplaySuffix(lastReplay[s].capturedLog, lastReplay[s].offset)

SessionDurableAcrossRuntimeDeath ==
  \A rk \in RuntimeKeys :
    runtimeIndex[rk].status = "stopped" =>
      \A s \in Sessions :
        IsPrefix(stopSnapshot[rk][s], sessionLog[s])

SessionScopedIdempotentAppend ==
  \A s \in Sessions :
    \A producerId \in ProducerIds :
      \A epoch \in ProducerEpochs :
        \A seq \in ProducerSeqs :
          Occurrences(sessionLog[s], producerId, epoch, seq) <= 1

HarnessEveryEffectLogged ==
  \A s \in Sessions :
    \A i \in 1..Len(visibleEffects[s]) :
      \E j \in 1..Len(sessionLog[s]) :
        /\ sessionLog[s][j].logicalId = visibleEffects[s][i].logicalId
        /\ sessionLog[s][j].kind = visibleEffects[s][i].kind

HarnessAppendOrderStable == SessionAppendOnly

HarnessSuspendReleasedOnlyByMatchingApproval ==
  \A req \in releasedRequests :
    FirstMatchingResolution(approvalHistory[req]) = "allow"

WakeOnReadyIsNoop ==
  lastWake.valid /\ lastWake.beforeStatus = "ready" =>
    /\ lastWake.createdNew = FALSE
    /\ lastWake.afterRuntimeId = lastWake.beforeRuntimeId

ConcurrentWakeSingleWinner ==
  /\ Cardinality(CurrentWakeResponses) <= 1
  /\ lastWake.valid =>
      \A c \in Callers :
        /\ responseEpochs[c] = wakeEpoch
        /\ wakeResponses[c] # NoRuntimeId
        => wakeResponses[c] = lastWake.afterRuntimeId

WakeOnStoppedChangesRuntimeId ==
  lastWake.valid /\ lastWake.createdNew =>
    /\ lastWake.beforeStatus = "stopped"
    /\ lastWake.afterRuntimeId # lastWake.beforeRuntimeId
    /\ runtimeIndex[lastWake.runtimeKey].runtimeId = lastWake.afterRuntimeId

\* Design property, not a checked invariant:
\* wake is derived from Session + Host state.
\* The spec intentionally carries no separate orchestration queue or
\* scheduler-owned state. Everything needed to decide wake lives in
\* `sessionLog`, `runtimeIndex`, `wakeEpoch`, and `wakeResponses`.

ProvisionReturnsReachableRuntime ==
  \A rk \in RuntimeKeys :
    runtimeIndex[rk].status = "ready" => reachable[rk]

ProvisionedRuntimeReusable ==
  WakeOnReadyIsNoop

WakeOnStoppedPreservesSessionBinding ==
  lastWake.valid /\ lastWake.createdNew =>
    /\ runtimeIndex[lastWake.runtimeKey].runtimeId = lastWake.afterRuntimeId
    /\ runtimeIndex[lastWake.runtimeKey].specPresent
    /\ lastWake.sessionId \in runtimeIndex[lastWake.runtimeKey].boundSessions

ResourceMountMappingCorrect ==
  \A rk \in RuntimeKeys :
    reachable[rk] =>
      mountedResources[runtimeIndex[rk].runtimeId] = requestedResources[rk]

FsBackendCapturesFsOpDurably ==
  \A s \in Sessions :
    \A i \in 1..Len(visibleEffects[s]) :
      visibleEffects[s][i].kind = "fs_op_captured" =>
        \E j \in 1..Len(sessionLog[s]) :
          /\ sessionLog[s][j].logicalId = visibleEffects[s][i].logicalId
          /\ sessionLog[s][j].kind = "fs_op_captured"

ToolDescriptorSchemaOnly ==
  \A t \in ToolNames :
    DOMAIN toolRegistry[t] = {"name", "description", "inputSchema"}

ToolDescriptorNoTransportLeak ==
  \A t \in ToolNames :
    ~("transportRef" \in DOMAIN toolRegistry[t])

ToolDescriptorNoCredentialLeak ==
  \A t \in ToolNames :
    ~("credentialRef" \in DOMAIN toolRegistry[t])

ToolRegistrationTransportAgnosticAtWireShape ==
  \A t \in ToolNames :
    toolRegistry[t] = capabilityRefs[t].descriptor

SandboxExecuteIsIsolatedFromHostRuntime ==
  lastAction = "sandbox_execute" =>
    sessionLog = previousSessionLog

SandboxCapabilityRefRespected ==
  \A sb \in SandboxIds :
    \A i \in 1..Len(sandboxToolHistory[sb]) :
      /\ sandboxToolHistory[sb][i] \in ToolNames
      /\ capabilityRefs[sandboxToolHistory[sb][i]].descriptor = toolRegistry[sandboxToolHistory[sb][i]]

SandboxStopDoesNotAffectHostRuntime ==
  lastAction = "sandbox_stop" =>
    runtimeIndex = previousRuntimeIndex

=============================================================================
