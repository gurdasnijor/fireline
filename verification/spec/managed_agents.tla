---- MODULE managed_agents ----
EXTENDS Naturals, Sequences, FiniteSets, TLC

\* Compact abstract model for Fireline's managed-agent substrate.
\*
\* Scope:
\* - Session as an append-only event log with offset replay and producer-tuple dedupe
\* - Orchestration as resume-by-composition over Session + Sandbox state
\* - Harness as "every visible effect lands in Session"
\* - Sandbox as provision/reuse/stop/reprovision of reachable runtimes
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
    "runtime_resumed",
    "session_loaded"
  }

DefaultSession == CHOOSE s \in Sessions : TRUE
DefaultRuntimeKey == CHOOSE rk \in RuntimeKeys : TRUE
DefaultRuntimeId == CHOOSE rid \in RuntimeIds : TRUE
NoRequest == "no_request"
NoRuntimeId == "no_runtime"

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

VARIABLES
  sessionLog,
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
  resumeEpoch,
  responseEpochs,
  lastResume,
  resumeResponses,
  logSnapshots

Vars ==
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
     resumeEpoch,
     responseEpochs,
     lastResume,
     resumeResponses,
     logSnapshots >>

Init ==
  /\ sessionLog = [s \in Sessions |-> <<>>]
  /\ runtimeIndex =
      [ rk \in RuntimeKeys |->
          [ status |-> "stopped",
            runtimeId |-> DefaultRuntimeId,
            specPresent |-> FALSE,
            boundSessions |-> {}
          ]
      ]
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
  /\ resumeEpoch = 0
  /\ responseEpochs = [c \in Callers |-> 0]
  /\ lastResume =
      [ valid |-> FALSE,
        caller |-> CHOOSE c \in Callers : TRUE,
        sessionId |-> DefaultSession,
        runtimeKey |-> DefaultRuntimeKey,
        beforeStatus |-> "stopped",
        beforeRuntimeId |-> DefaultRuntimeId,
        afterRuntimeId |-> DefaultRuntimeId,
        createdNew |-> FALSE
      ]
  /\ resumeResponses = [c \in Callers |-> NoRuntimeId]
  /\ logSnapshots = << [s \in Sessions |-> <<>>] >>

AppendSnapshot(nextLog) ==
  logSnapshots' = Append(logSnapshots, nextLog)

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
  /\ UNCHANGED
      << sessionLog,
         pendingApprovals,
         toolRegistry,
         capabilityRefs,
         seenCommits,
         approvalHistory,
         visibleEffects,
         blockedRequests,
         releasedRequests,
         lastReplay,
         stopSnapshot,
         resumeEpoch,
         responseEpochs,
         lastResume,
         resumeResponses >>
  /\ AppendSnapshot(sessionLog)

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
  /\ UNCHANGED
      << runtimeIndex,
         pendingApprovals,
         toolRegistry,
         capabilityRefs,
         requestedResources,
         mountedResources,
         approvalHistory,
         blockedRequests,
         releasedRequests,
         reachable,
         lastReplay,
         stopSnapshot,
         resumeEpoch,
         responseEpochs,
         lastResume,
         resumeResponses >>
  /\ AppendSnapshot(sessionLog')

RetryAppend(s, producerId, epoch, seq) ==
  /\ CommitTuple(producerId, epoch, seq) \in seenCommits[s]
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
         resumeEpoch,
         responseEpochs,
         lastResume,
         resumeResponses >>
  /\ AppendSnapshot(sessionLog)

ReplayFromOffset(s, offset) ==
  /\ offset \in 0..Len(sessionLog[s])
  /\ lastReplay' =
      [lastReplay EXCEPT ![s] =
        [ offset |-> offset,
          capturedLog |-> sessionLog[s],
          suffix |-> ReplaySuffix(sessionLog[s], offset)
        ]
      ]
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
         stopSnapshot,
         resumeEpoch,
         responseEpochs,
         lastResume,
         resumeResponses >>
  /\ AppendSnapshot(sessionLog)

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
  /\ UNCHANGED
      << runtimeIndex,
         toolRegistry,
         capabilityRefs,
         requestedResources,
         mountedResources,
         approvalHistory,
         visibleEffects,
         releasedRequests,
         reachable,
         lastReplay,
         stopSnapshot,
         resumeEpoch,
         responseEpochs,
         lastResume,
         resumeResponses >>
  /\ AppendSnapshot(sessionLog')

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
  /\ UNCHANGED
      << runtimeIndex,
         toolRegistry,
         capabilityRefs,
         requestedResources,
         mountedResources,
         visibleEffects,
         blockedRequests,
         releasedRequests,
         reachable,
         lastReplay,
         stopSnapshot,
         resumeEpoch,
         responseEpochs,
         lastResume,
         resumeResponses >>
  /\ AppendSnapshot(sessionLog')

AdvanceBlockedRequest(s, req) ==
  /\ blockedRequests[s] = req
  /\ FirstMatchingResolution(approvalHistory[req]) = "allow"
  /\ blockedRequests' = [blockedRequests EXCEPT ![s] = NoRequest]
  /\ releasedRequests' = releasedRequests \cup {req}
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
         reachable,
         lastReplay,
         stopSnapshot,
         resumeEpoch,
         responseEpochs,
         lastResume,
         resumeResponses >>
  /\ AppendSnapshot(sessionLog)

StopRuntime(rk) ==
  /\ runtimeIndex[rk].status = "ready"
  /\ runtimeIndex' = [runtimeIndex EXCEPT ![rk].status = "stopped"]
  /\ reachable' = [reachable EXCEPT ![rk] = FALSE]
  /\ stopSnapshot' = [stopSnapshot EXCEPT ![rk] = sessionLog]
  /\ UNCHANGED
      << sessionLog,
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
         lastReplay,
         resumeEpoch,
         responseEpochs,
         lastResume,
         resumeResponses >>
  /\ AppendSnapshot(sessionLog)

ResumeLive(caller, s, rk) ==
  LET sameEpisode ==
        /\ lastResume.valid
        /\ lastResume.sessionId = s
        /\ lastResume.runtimeKey = rk
        /\ lastResume.beforeStatus = "ready"
        /\ lastResume.afterRuntimeId = runtimeIndex[rk].runtimeId
        /\ lastResume.createdNew = FALSE
      newEpoch ==
        IF sameEpisode THEN resumeEpoch ELSE resumeEpoch + 1
  IN
  /\ runtimeIndex[rk].status = "ready"
  /\ s \in runtimeIndex[rk].boundSessions
  /\ resumeEpoch' = newEpoch
  /\ responseEpochs' =
      [ c \in Callers |->
          IF sameEpisode THEN
            IF c = caller THEN newEpoch ELSE responseEpochs[c]
          ELSE
            IF c = caller THEN newEpoch ELSE 0
      ]
  /\ lastResume' =
      [ valid |-> TRUE,
        caller |-> caller,
        sessionId |-> s,
        runtimeKey |-> rk,
        beforeStatus |-> "ready",
        beforeRuntimeId |-> runtimeIndex[rk].runtimeId,
        afterRuntimeId |-> runtimeIndex[rk].runtimeId,
        createdNew |-> FALSE
      ]
  /\ resumeResponses' =
      [ c \in Callers |->
          IF sameEpisode THEN
            IF c = caller THEN runtimeIndex[rk].runtimeId ELSE resumeResponses[c]
          ELSE
            IF c = caller THEN runtimeIndex[rk].runtimeId ELSE NoRuntimeId
      ]
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
         stopSnapshot >>
  /\ AppendSnapshot(sessionLog)

ResumeCold(caller, s, rk, newRid) ==
  LET sameEpisode ==
        /\ lastResume.valid
        /\ lastResume.sessionId = s
        /\ lastResume.runtimeKey = rk
        /\ lastResume.beforeStatus = "stopped"
        /\ lastResume.afterRuntimeId = newRid
        /\ lastResume.createdNew = TRUE
      newEpoch ==
        IF sameEpisode THEN resumeEpoch ELSE resumeEpoch + 1
  IN
  /\ runtimeIndex[rk].status = "stopped"
  /\ runtimeIndex[rk].specPresent
  /\ s \in runtimeIndex[rk].boundSessions
  /\ newRid # runtimeIndex[rk].runtimeId
  /\ resumeEpoch' = newEpoch
  /\ responseEpochs' =
      [ c \in Callers |->
          IF sameEpisode THEN
            IF c = caller THEN newEpoch ELSE responseEpochs[c]
          ELSE
            IF c = caller THEN newEpoch ELSE 0
      ]
  /\ runtimeIndex' =
      [runtimeIndex EXCEPT ![rk] =
        [ @ EXCEPT !.status = "ready", !.runtimeId = newRid ]
      ]
  /\ mountedResources' = [mountedResources EXCEPT ![newRid] = requestedResources[rk]]
  /\ reachable' = [reachable EXCEPT ![rk] = TRUE]
  /\ lastResume' =
      [ valid |-> TRUE,
        caller |-> caller,
        sessionId |-> s,
        runtimeKey |-> rk,
        beforeStatus |-> "stopped",
        beforeRuntimeId |-> runtimeIndex[rk].runtimeId,
        afterRuntimeId |-> newRid,
        createdNew |-> TRUE
      ]
  /\ resumeResponses' =
      [ c \in Callers |->
          IF sameEpisode THEN
            IF c = caller THEN newRid ELSE resumeResponses[c]
          ELSE
            IF c = caller THEN newRid ELSE NoRuntimeId
      ]
  /\ UNCHANGED
      << sessionLog,
         pendingApprovals,
         toolRegistry,
         capabilityRefs,
         requestedResources,
         seenCommits,
         approvalHistory,
         visibleEffects,
         blockedRequests,
         releasedRequests,
         lastReplay,
         stopSnapshot >>
  /\ AppendSnapshot(sessionLog)

CurrentResumeResponses ==
  { resumeResponses[c] :
      c \in { d \in Callers :
                /\ responseEpochs[d] = resumeEpoch
                /\ resumeResponses[d] # NoRuntimeId
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
  \/ \E caller \in Callers, s \in Sessions, rk \in RuntimeKeys :
       ResumeLive(caller, s, rk)
  \/ \E caller \in Callers, s \in Sessions, rk \in RuntimeKeys, newRid \in RuntimeIds :
       ResumeCold(caller, s, rk, newRid)

Spec == Init /\ [][Next]_Vars

SessionAppendOnly ==
  \A i, j \in 1..Len(logSnapshots) :
    i <= j =>
      \A s \in Sessions :
        IsPrefix(logSnapshots[i][s], logSnapshots[j][s])

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

ResumeOnLiveRuntimeIsNoop ==
  lastResume.valid /\ lastResume.beforeStatus = "ready" =>
    /\ lastResume.createdNew = FALSE
    /\ lastResume.afterRuntimeId = lastResume.beforeRuntimeId

ConcurrentResumeSingleWinner ==
  /\ Cardinality(CurrentResumeResponses) <= 1
  /\ lastResume.valid =>
      \A c \in Callers :
        /\ responseEpochs[c] = resumeEpoch
        /\ resumeResponses[c] # NoRuntimeId
        => resumeResponses[c] = lastResume.afterRuntimeId

ColdResumePreservesRuntimeKeyChangesRuntimeId ==
  lastResume.valid /\ lastResume.createdNew =>
    /\ lastResume.beforeStatus = "stopped"
    /\ lastResume.afterRuntimeId # lastResume.beforeRuntimeId
    /\ runtimeIndex[lastResume.runtimeKey].runtimeId = lastResume.afterRuntimeId

\* Design property, not a checked invariant:
\* resume is derived from Session + Sandbox state.
\* The spec intentionally carries no separate orchestration queue or
\* scheduler-owned state. Everything needed to decide resume lives in
\* `sessionLog`, `runtimeIndex`, `resumeEpoch`, and `resumeResponses`.

ProvisionReturnsReachableRuntime ==
  \A rk \in RuntimeKeys :
    runtimeIndex[rk].status = "ready" => reachable[rk]

ProvisionedRuntimeReusable ==
  ResumeOnLiveRuntimeIsNoop

ReprovisionPreservesLoadSessionSemantics ==
  lastResume.valid /\ lastResume.createdNew =>
    /\ runtimeIndex[lastResume.runtimeKey].runtimeId = lastResume.afterRuntimeId
    /\ runtimeIndex[lastResume.runtimeKey].specPresent
    /\ lastResume.sessionId \in runtimeIndex[lastResume.runtimeKey].boundSessions

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

=============================================================================
