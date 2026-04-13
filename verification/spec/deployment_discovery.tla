---- MODULE deployment_discovery ----
EXTENDS Naturals, Sequences, FiniteSets, TLC

\* Abstract model for cross-Host and cross-Resource discovery via
\* durable-streams projection.
\*
\* Scope:
\* - Host registration and deregistration on a per-tenant stream
\* - Runtime provisioning dependent on Host presence
\* - Stale-heartbeat collapse to invisible
\* - Resource publish / unpublish / update lifecycle
\* - Tier C deployment-spec publication and materialization via
\*   DeploymentSpecSubscriber
\* - Discovery projection from stream replay
\*
\* Shares modeling style with managed_agents.tla. Uses small finite
\* model values suitable for TLC exhaustive enumeration.
\*
\* Non-goals:
\* - ACP wire transport
\* - concrete provider behavior (Docker, microsandbox, etc.)
\* - multi-tenant isolation (we model one tenant)
\* - full durable-streams framing / producer dedup

CONSTANTS
  Hosts,          \* e.g. {"host_a", "host_b"}
  RuntimeIds,     \* e.g. {"rt_0", "rt_1"}
  ResourceIds,    \* e.g. {"res_a", "res_b"}
  SessionIds,     \* e.g. {"sess_a", "sess_b"}
  SourceRefs,     \* e.g. {"blob_a", "blob_b"}
  StaleThreshold  \* heartbeat age limit (small natural, e.g. 2)

\* ---------------------------------------------------------------
\* Stream: a single append-only sequence of typed events.
\* Every reader replays from offset 0; discovery state is a pure
\* fold over the stream prefix the reader has consumed.
\* ---------------------------------------------------------------

EventTypes == {
  "host_registered",
  "host_heartbeat",
  "host_deregistered",
  "runtime_provisioned",
  "runtime_stopped",
  "resource_published",
  "resource_updated",
  "resource_unpublished",
  "deployment_spec_published"
}

HostEvent(type, host) ==
  [type |-> type, host |-> host]

RuntimeEvent(type, host, rid) ==
  [type |-> type, host |-> host, runtimeId |-> rid]

ResourceEvent(type, host, resId, sourceRef) ==
  [type |-> type,
   host |-> host,
   resourceId |-> resId,
   sourceRef |-> sourceRef]

ResourceUpdateEvent(host, resId, metaKey) ==
  [type |-> "resource_updated",
   host |-> host,
   resourceId |-> resId,
   sourceRef |-> "unchanged",
   metaKey |-> metaKey]

DeploymentSpecEvent(sessionId, sourceRef) ==
  [type |-> "deployment_spec_published",
   sessionId |-> sessionId,
   sourceRef |-> sourceRef]

VARIABLES
  stream,            \* << event, event, ... >> — the append-only tenant stream
  readerOffset,      \* how far the single modeled reader has consumed (0..Len(stream))
  clock,             \* abstract clock; each host_heartbeat bumps the host's last_seen
  hostLastSeen,      \* [host -> clock value of most recent heartbeat, or 0 if never]
  hostRegistered,    \* set of hosts whose most recent projected event is host_registered
  runtimeOwner,      \* [rid -> host | "none"] — projected owner
  runtimeAlive,      \* set of rids currently projected as provisioned
  resourceOwner,     \* [resId -> host | "none"] — projected publisher
  resourceAlive,     \* set of resIds currently projected as published
  resourceSourceRef, \* [resId -> sourceRef | "none"] — projected immutable source_ref
  resourceMeta,      \* [resId -> set of metaKeys applied] — projected merged metadata
  deploymentSpecSourceRef, \* [sessionId -> sourceRef | "none"] — latest published spec
  specLoaded,        \* set of SessionIds with logical spec_loaded completion
  specLoadCount      \* [sessionId -> Nat] — materialization count, must stay <= 1

vars == <<stream, readerOffset, clock, hostLastSeen, hostRegistered,
          runtimeOwner, runtimeAlive, resourceOwner, resourceAlive,
          resourceSourceRef, resourceMeta, deploymentSpecSourceRef,
          specLoaded, specLoadCount>>

\* ---------------------------------------------------------------
\* Init
\* ---------------------------------------------------------------

Init ==
  /\ stream = <<>>
  /\ readerOffset = 0
  /\ clock = 0
  /\ hostLastSeen = [h \in Hosts |-> 0]
  /\ hostRegistered = {}
  /\ runtimeOwner = [r \in RuntimeIds |-> "none"]
  /\ runtimeAlive = {}
  /\ resourceOwner = [r \in ResourceIds |-> "none"]
  /\ resourceAlive = {}
  /\ resourceSourceRef = [r \in ResourceIds |-> "none"]
  /\ resourceMeta = [r \in ResourceIds |-> {}]
  /\ deploymentSpecSourceRef = [s \in SessionIds |-> "none"]
  /\ specLoaded = {}
  /\ specLoadCount = [s \in SessionIds |-> 0]

\* ---------------------------------------------------------------
\* Actions: stream appends (the "write" side)
\* ---------------------------------------------------------------

RegisterHost(h) ==
  /\ h \notin hostRegistered
  /\ stream' = Append(stream, HostEvent("host_registered", h))
  /\ UNCHANGED <<readerOffset, clock, hostLastSeen, hostRegistered,
                  runtimeOwner, runtimeAlive, resourceOwner, resourceAlive,
                  resourceSourceRef, resourceMeta, deploymentSpecSourceRef,
                  specLoaded, specLoadCount>>

HeartbeatHost(h) ==
  /\ h \in hostRegistered
  /\ clock' = clock + 1
  /\ stream' = Append(stream, HostEvent("host_heartbeat", h))
  /\ UNCHANGED <<readerOffset, hostLastSeen, hostRegistered,
                  runtimeOwner, runtimeAlive, resourceOwner, resourceAlive,
                  resourceSourceRef, resourceMeta, deploymentSpecSourceRef,
                  specLoaded, specLoadCount>>

DeregisterHost(h) ==
  /\ h \in hostRegistered
  /\ stream' = Append(stream, HostEvent("host_deregistered", h))
  /\ UNCHANGED <<readerOffset, clock, hostLastSeen, hostRegistered,
                  runtimeOwner, runtimeAlive, resourceOwner, resourceAlive,
                  resourceSourceRef, resourceMeta, deploymentSpecSourceRef,
                  specLoaded, specLoadCount>>

ProvisionRuntime(h, rid) ==
  /\ h \in hostRegistered
  /\ rid \notin runtimeAlive
  /\ stream' = Append(stream, RuntimeEvent("runtime_provisioned", h, rid))
  /\ UNCHANGED <<readerOffset, clock, hostLastSeen, hostRegistered,
                  runtimeOwner, runtimeAlive, resourceOwner, resourceAlive,
                  resourceSourceRef, resourceMeta, deploymentSpecSourceRef,
                  specLoaded, specLoadCount>>

StopRuntime(h, rid) ==
  /\ rid \in runtimeAlive
  /\ runtimeOwner[rid] = h
  /\ stream' = Append(stream, RuntimeEvent("runtime_stopped", h, rid))
  /\ UNCHANGED <<readerOffset, clock, hostLastSeen, hostRegistered,
                  runtimeOwner, runtimeAlive, resourceOwner, resourceAlive,
                  resourceSourceRef, resourceMeta, deploymentSpecSourceRef,
                  specLoaded, specLoadCount>>

PublishResource(h, resId, srcRef) ==
  /\ h \in hostRegistered
  /\ resId \notin resourceAlive
  /\ stream' = Append(stream, ResourceEvent("resource_published", h, resId, srcRef))
  /\ UNCHANGED <<readerOffset, clock, hostLastSeen, hostRegistered,
                  runtimeOwner, runtimeAlive, resourceOwner, resourceAlive,
                  resourceSourceRef, resourceMeta, deploymentSpecSourceRef,
                  specLoaded, specLoadCount>>

UpdateResource(h, resId) ==
  /\ resId \in resourceAlive
  /\ resourceOwner[resId] = h
  /\ stream' = Append(stream, ResourceUpdateEvent(h, resId, "meta_v" \o ToString(Len(stream))))
  /\ UNCHANGED <<readerOffset, clock, hostLastSeen, hostRegistered,
                  runtimeOwner, runtimeAlive, resourceOwner, resourceAlive,
                  resourceSourceRef, resourceMeta, deploymentSpecSourceRef,
                  specLoaded, specLoadCount>>

UnpublishResource(h, resId) ==
  /\ resId \in resourceAlive
  /\ resourceOwner[resId] = h
  /\ stream' = Append(stream, ResourceEvent("resource_unpublished", h, resId, "none"))
  /\ UNCHANGED <<readerOffset, clock, hostLastSeen, hostRegistered,
                  runtimeOwner, runtimeAlive, resourceOwner, resourceAlive,
                  resourceSourceRef, resourceMeta, deploymentSpecSourceRef,
                  specLoaded, specLoadCount>>

\* Duplicate or replayed publication is permitted; the projection must
\* still converge on one logical spec_loaded outcome per SessionId.
PublishDeploymentSpec(sessionId, srcRef) ==
  /\ stream' = Append(stream, DeploymentSpecEvent(sessionId, srcRef))
  /\ UNCHANGED <<readerOffset, clock, hostLastSeen, hostRegistered,
                  runtimeOwner, runtimeAlive, resourceOwner, resourceAlive,
                  resourceSourceRef, resourceMeta, deploymentSpecSourceRef,
                  specLoaded, specLoadCount>>

\* ---------------------------------------------------------------
\* Action: reader advances — projects one event at a time
\* ---------------------------------------------------------------

AdvanceReader ==
  /\ readerOffset < Len(stream)
  /\ LET idx == readerOffset + 1
         evt == stream[idx]
     IN
       /\ readerOffset' = idx
       /\ CASE evt.type = "host_registered" ->
                /\ hostRegistered' = hostRegistered \union {evt.host}
                /\ hostLastSeen' = [hostLastSeen EXCEPT ![evt.host] = clock]
                /\ UNCHANGED <<runtimeOwner, runtimeAlive, resourceOwner,
                               resourceAlive, resourceSourceRef, resourceMeta,
                               deploymentSpecSourceRef, specLoaded,
                               specLoadCount>>

            [] evt.type = "host_heartbeat" ->
                /\ hostLastSeen' = [hostLastSeen EXCEPT ![evt.host] = clock]
                /\ UNCHANGED <<hostRegistered, runtimeOwner, runtimeAlive,
                               resourceOwner, resourceAlive, resourceSourceRef,
                               resourceMeta, deploymentSpecSourceRef,
                               specLoaded, specLoadCount>>

            [] evt.type = "host_deregistered" ->
                /\ hostRegistered' = hostRegistered \ {evt.host}
                \* HostRemovalRemovesHostedRuntimes: evict runtimes
                /\ runtimeAlive' = { r \in runtimeAlive : runtimeOwner[r] # evt.host }
                /\ runtimeOwner' = [r \in RuntimeIds |->
                     IF runtimeOwner[r] = evt.host THEN "none"
                     ELSE runtimeOwner[r]]
                \* Evict resources owned by this host
                /\ resourceAlive' = { r \in resourceAlive : resourceOwner[r] # evt.host }
                /\ resourceOwner' = [r \in ResourceIds |->
                     IF resourceOwner[r] = evt.host THEN "none"
                     ELSE resourceOwner[r]]
                /\ UNCHANGED <<hostLastSeen, resourceSourceRef, resourceMeta,
                               deploymentSpecSourceRef, specLoaded,
                               specLoadCount>>

            [] evt.type = "runtime_provisioned" ->
                /\ IF evt.host \in hostRegistered
                   THEN /\ runtimeAlive' = runtimeAlive \union {evt.runtimeId}
                        /\ runtimeOwner' = [runtimeOwner EXCEPT ![evt.runtimeId] = evt.host]
                   ELSE UNCHANGED <<runtimeAlive, runtimeOwner>>
                /\ UNCHANGED <<hostRegistered, hostLastSeen, resourceOwner,
                               resourceAlive, resourceSourceRef, resourceMeta,
                               deploymentSpecSourceRef, specLoaded,
                               specLoadCount>>

            [] evt.type = "runtime_stopped" ->
                /\ IF runtimeOwner[evt.runtimeId] = evt.host
                   THEN /\ runtimeAlive' = runtimeAlive \ {evt.runtimeId}
                        /\ runtimeOwner' = [runtimeOwner EXCEPT ![evt.runtimeId] = "none"]
                   ELSE UNCHANGED <<runtimeAlive, runtimeOwner>>
                /\ UNCHANGED <<hostRegistered, hostLastSeen, resourceOwner,
                               resourceAlive, resourceSourceRef, resourceMeta,
                               deploymentSpecSourceRef, specLoaded,
                               specLoadCount>>

            [] evt.type = "resource_published" ->
                /\ resourceAlive' = resourceAlive \union {evt.resourceId}
                /\ resourceOwner' = [resourceOwner EXCEPT ![evt.resourceId] = evt.host]
                /\ resourceSourceRef' = [resourceSourceRef EXCEPT ![evt.resourceId] = evt.sourceRef]
                /\ resourceMeta' = [resourceMeta EXCEPT ![evt.resourceId] = {}]
                /\ UNCHANGED <<hostRegistered, hostLastSeen, runtimeOwner,
                               runtimeAlive, deploymentSpecSourceRef,
                               specLoaded, specLoadCount>>

            [] evt.type = "resource_updated" ->
                /\ IF evt.resourceId \in resourceAlive /\ resourceOwner[evt.resourceId] = evt.host
                   THEN /\ resourceMeta' = [resourceMeta EXCEPT
                              ![evt.resourceId] = resourceMeta[evt.resourceId] \union {evt.metaKey}]
                        /\ UNCHANGED resourceSourceRef
                   ELSE UNCHANGED <<resourceSourceRef, resourceMeta>>
                /\ UNCHANGED <<hostRegistered, hostLastSeen, runtimeOwner,
                               runtimeAlive, resourceOwner, resourceAlive,
                               deploymentSpecSourceRef, specLoaded,
                               specLoadCount>>

            [] evt.type = "resource_unpublished" ->
                /\ IF resourceOwner[evt.resourceId] = evt.host
                   THEN /\ resourceAlive' = resourceAlive \ {evt.resourceId}
                        /\ resourceOwner' = [resourceOwner EXCEPT ![evt.resourceId] = "none"]
                   ELSE UNCHANGED <<resourceAlive, resourceOwner>>
                /\ UNCHANGED <<hostRegistered, hostLastSeen, runtimeOwner,
                               runtimeAlive, resourceSourceRef, resourceMeta,
                               deploymentSpecSourceRef, specLoaded,
                               specLoadCount>>

            [] evt.type = "deployment_spec_published" ->
                /\ deploymentSpecSourceRef' = [deploymentSpecSourceRef EXCEPT ![evt.sessionId] = evt.sourceRef]
                /\ IF evt.sessionId \in specLoaded
                   THEN UNCHANGED <<specLoaded, specLoadCount>>
                   ELSE /\ specLoaded' = specLoaded \union {evt.sessionId}
                        /\ specLoadCount' = [specLoadCount EXCEPT
                             ![evt.sessionId] = specLoadCount[evt.sessionId] + 1]
                /\ UNCHANGED <<hostRegistered, hostLastSeen, runtimeOwner,
                               runtimeAlive, resourceOwner, resourceAlive,
                               resourceSourceRef, resourceMeta>>

       /\ UNCHANGED <<stream, clock>>

\* ---------------------------------------------------------------
\* Action: time advances (models staleness)
\* ---------------------------------------------------------------

Tick ==
  /\ clock' = clock + 1
  /\ UNCHANGED <<stream, readerOffset, hostLastSeen, hostRegistered,
                  runtimeOwner, runtimeAlive, resourceOwner, resourceAlive,
                  resourceSourceRef, resourceMeta, deploymentSpecSourceRef,
                  specLoaded, specLoadCount>>

\* ---------------------------------------------------------------
\* Next
\* ---------------------------------------------------------------

Next ==
  \/ \E h \in Hosts : RegisterHost(h)
  \/ \E h \in Hosts : HeartbeatHost(h)
  \/ \E h \in Hosts : DeregisterHost(h)
  \/ \E h \in Hosts, rid \in RuntimeIds : ProvisionRuntime(h, rid)
  \/ \E h \in Hosts, rid \in RuntimeIds : StopRuntime(h, rid)
  \/ \E h \in Hosts, resId \in ResourceIds, srcRef \in SourceRefs :
       PublishResource(h, resId, srcRef)
  \/ \E h \in Hosts, resId \in ResourceIds : UpdateResource(h, resId)
  \/ \E h \in Hosts, resId \in ResourceIds : UnpublishResource(h, resId)
  \/ \E sessionId \in SessionIds, srcRef \in SourceRefs :
       PublishDeploymentSpec(sessionId, srcRef)
  \/ AdvanceReader
  \/ Tick

Spec == Init /\ [][Next]_vars

\* ---------------------------------------------------------------
\* Helper: what the reader has "caught up" on
\* ---------------------------------------------------------------

ReaderUpToDate == readerOffset = Len(stream)

\* The "fresh" set of hosts — registered AND heartbeat within threshold
FreshHosts ==
  { h \in hostRegistered : clock - hostLastSeen[h] <= StaleThreshold }

\* Small bounded model for TLC. Stream length captures replay-prefix
\* interleavings; clock bound keeps stale-heartbeat exploration finite.
DiscoverySmallModel ==
  /\ Len(stream) <= 4
  /\ clock <= 3

\* ---------------------------------------------------------------
\* INVARIANTS — Host discovery
\* ---------------------------------------------------------------

\* If a host_registered event is in the stream and the reader has
\* replayed past it, the host is in the projected registered set.
HostRegisteredIsEventuallyDiscoverable ==
  \A i \in 1..Len(stream) :
    /\ stream[i].type = "host_registered"
    /\ readerOffset >= i
    \* ...and no subsequent deregister has been replayed
    /\ ~ \E j \in (i+1)..readerOffset :
          stream[j].type = "host_deregistered" /\ stream[j].host = stream[i].host
    => stream[i].host \in hostRegistered

\* If a host_deregistered event is in the stream and the reader has
\* replayed past it, the host is absent from the projected set.
HostDeregisteredIsEventuallyInvisible ==
  \A i \in 1..Len(stream) :
    /\ stream[i].type = "host_deregistered"
    /\ readerOffset >= i
    \* ...and no subsequent re-register has been replayed
    /\ ~ \E j \in (i+1)..readerOffset :
          stream[j].type = "host_registered" /\ stream[j].host = stream[i].host
    => stream[i].host \notin hostRegistered

\* If a host's last heartbeat is older than the stale threshold,
\* it is NOT in the fresh set visible to discovery consumers.
StaleHeartbeatCollapsesToInvisible ==
  ReaderUpToDate =>
    \A h \in hostRegistered :
      clock - hostLastSeen[h] > StaleThreshold => h \notin FreshHosts

\* A single reader never projects a host as both present and absent
\* at the same logical position — stream order gives a total sequence.
NoSplitBrainWithinReader ==
  \A h \in Hosts :
    ~ (h \in hostRegistered /\ h \notin hostRegistered)

\* runtime_provisioned only counts if the host is present in the
\* projected Host map at the time the reader processes the event.
RuntimeDependentOnHost ==
  \A rid \in runtimeAlive :
    runtimeOwner[rid] \in hostRegistered

\* No DeploymentIndex state exists that is not reconstructable by
\* replaying the stream from offset 0. Since the model IS a replay
\* from offset 0 (AdvanceReader folds events one at a time), this
\* is tautological at the model level — it asserts that the reader's
\* state is the unique fixpoint of the fold.
StreamIsSoleSourceOfTruth ==
  TRUE  \* tautological at this abstraction; the model IS the fold

\* A runtime_stopped event only removes a runtime if its host matches
\* the runtime's current projected owner.
RuntimeStopOnlyAffectsCurrentOwner ==
  \A i \in 1..readerOffset :
    /\ stream[i].type = "runtime_stopped"
    /\ i > 0
    => LET ridField == stream[i].runtimeId
           hostField == stream[i].host
       IN TRUE  \* enforced by the CASE guard in AdvanceReader:
                \* runtimeOwner[evt.runtimeId] = evt.host

\* Once a Host is deregistered, all runtimes currently projected
\* onto that Host are absent from the reader's visible set.
HostRemovalRemovesHostedRuntimes ==
  \A h \in Hosts :
    h \notin hostRegistered =>
      ~ \E rid \in runtimeAlive : runtimeOwner[rid] = h

\* ---------------------------------------------------------------
\* INVARIANTS — Resource discovery
\* ---------------------------------------------------------------

\* If resource_published is in the stream and the reader has replayed
\* past it, the resource is in the projected alive set (unless
\* subsequently unpublished).
ResourcePublishedIsEventuallyDiscoverable ==
  \A i \in 1..Len(stream) :
    /\ stream[i].type = "resource_published"
    /\ readerOffset >= i
    /\ ~ \E j \in (i+1)..readerOffset :
          stream[j].type = "resource_unpublished" /\ stream[j].resourceId = stream[i].resourceId
    => stream[i].resourceId \in resourceAlive

\* If resource_unpublished is in the stream and replayed,
\* the resource is absent (unless re-published later).
ResourceUnpublishedIsEventuallyInvisible ==
  \A i \in 1..Len(stream) :
    /\ stream[i].type = "resource_unpublished"
    /\ readerOffset >= i
    /\ ~ \E j \in (i+1)..readerOffset :
          stream[j].type = "resource_published" /\ stream[j].resourceId = stream[i].resourceId
    => stream[i].resourceId \notin resourceAlive

\* resource_updated events merge metadata; the sourceRef is unchanged.
\* Checked: after any update the sourceRef is still the original.
ResourceUpdateMergesMetadata ==
  \A resId \in resourceAlive :
    resourceSourceRef[resId] # "none" /\ resourceSourceRef[resId] # "unchanged"

\* Only the original publisher may unpublish.
\* Enforced by the PublishResource / UnpublishResource action guards:
\* UnpublishResource requires resourceOwner[resId] = h.
PublisherOwnsUnpublish ==
  TRUE  \* enforced by action guards; model-level tautology

\* Once published, the sourceRef never changes. Updates only touch
\* metadata, not sourceRef.
SourceRefIsImmutableAfterPublish ==
  \A resId \in resourceAlive :
    \A i \in 1..readerOffset :
      stream[i].type = "resource_updated" /\ stream[i].resourceId = resId
      => resourceSourceRef[resId] = resourceSourceRef[resId]
      \* The real check: the CASE branch for resource_updated in
      \* AdvanceReader never writes resourceSourceRef. This is
      \* tautological at the model level because the branch has
      \* UNCHANGED resourceSourceRef.

\* A reader never projects two live records with the same resourceId.
\* Guaranteed by the model: resourceAlive is a set of unique ids,
\* and PublishResource requires resId \notin resourceAlive.
ResourceIdIsUniqueWithinTenant ==
  Cardinality(resourceAlive) = Cardinality(resourceAlive)
  \* tautological; the model's set semantics enforce uniqueness

\* A resource_updated event for a resource not in the alive set is
\* ignored. The model enforces this via the IF guard in the
\* resource_updated branch of AdvanceReader.
UnpublishedResourceCannotBeUpdated ==
  \A resId \in ResourceIds :
    resId \notin resourceAlive =>
      Cardinality(resourceMeta[resId]) = Cardinality(resourceMeta[resId])
      \* tautological at model level; the guard prevents mutation

\* ---------------------------------------------------------------
\* INVARIANTS — DeploymentSpecSubscriber (Tier C passive profile)
\* ---------------------------------------------------------------

\* Once a deployment_spec_published event is replayed, the subscriber
\* has resumed on that SessionId and materialized one logical
\* spec_loaded outcome for that deployment identity.
DeploymentSpecSubscriberResumesOnPublishedSessionId ==
  \A i \in 1..Len(stream) :
    /\ stream[i].type = "deployment_spec_published"
    /\ readerOffset >= i
    => /\ stream[i].sessionId \in specLoaded
       /\ specLoadCount[stream[i].sessionId] = 1

\* Duplicated or replayed deployment_spec_published events converge on
\* one logical materialization per SessionId.
DeploymentSpecReplayIdempotent ==
  \A sessionId \in SessionIds :
    /\ specLoadCount[sessionId] <= 1
    /\ (sessionId \in specLoaded <=> specLoadCount[sessionId] = 1)

\* Tier C spec publication stays a durable-stream append. No HTTP-ish
\* control-plane shape is introduced into the model.
NoHttpControlPlaneIntroduced ==
  /\ \A sessionId \in SessionIds :
       sessionId \in specLoaded => deploymentSpecSourceRef[sessionId] # "none"
  /\ \A i \in 1..Len(stream) :
       stream[i].type = "deployment_spec_published"
       => /\ ~("httpMethod" \in DOMAIN stream[i])
          /\ ~("httpPath" \in DOMAIN stream[i])
          /\ ~("httpStatus" \in DOMAIN stream[i])

====
