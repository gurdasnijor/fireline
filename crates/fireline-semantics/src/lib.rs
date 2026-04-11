pub mod conductor;

pub mod liveness {
    #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
    pub enum RuntimeKey {
        A,
        B,
    }

    impl RuntimeKey {
        pub const ALL: [RuntimeKey; 2] = [RuntimeKey::A, RuntimeKey::B];

        pub const fn index(self) -> usize {
            match self {
                RuntimeKey::A => 0,
                RuntimeKey::B => 1,
            }
        }
    }

    #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
    pub enum BaseRuntimeStatus {
        Ready,
        Stopped,
    }

    #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
    pub enum HeartbeatFreshness {
        Unknown,
        Fresh,
        Stale,
    }

    #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
    pub enum ObservableRuntimeStatus {
        Ready,
        Stale,
        Stopped,
    }

    #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
    pub struct RuntimeLivenessRecord {
        pub base_status: BaseRuntimeStatus,
        pub last_seen_at: Option<u64>,
    }

    impl Default for RuntimeLivenessRecord {
        fn default() -> Self {
            Self {
                base_status: BaseRuntimeStatus::Stopped,
                last_seen_at: None,
            }
        }
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq)]
    pub struct RegistryLivenessState {
        pub now: u64,
        pub stale_timeout: u64,
        pub runtimes: [RuntimeLivenessRecord; 2],
        pub last_scan_at: Option<u64>,
    }

    impl Default for RegistryLivenessState {
        fn default() -> Self {
            Self {
                now: 0,
                stale_timeout: 1,
                runtimes: [
                    RuntimeLivenessRecord::default(),
                    RuntimeLivenessRecord::default(),
                ],
                last_scan_at: None,
            }
        }
    }

    #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
    pub enum RegistryLivenessAction {
        Register { runtime: RuntimeKey },
        Heartbeat { runtime: RuntimeKey },
        AdvanceTime,
        StaleScan,
    }

    #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
    pub enum RegistryLivenessTransition {
        Registered { runtime: RuntimeKey, at: u64 },
        HeartbeatRecorded { runtime: RuntimeKey, at: u64 },
        TimeAdvanced { now: u64 },
        Scanned { at: u64 },
    }

    impl RegistryLivenessState {
        pub fn record(&self, runtime: RuntimeKey) -> RuntimeLivenessRecord {
            self.runtimes[runtime.index()]
        }

        pub fn heartbeat_freshness(&self, runtime: RuntimeKey) -> HeartbeatFreshness {
            let record = self.record(runtime);
            match (record.base_status, record.last_seen_at) {
                (BaseRuntimeStatus::Stopped, _) => HeartbeatFreshness::Unknown,
                (_, None) => HeartbeatFreshness::Unknown,
                (_, Some(last_seen_at)) => {
                    if self.now.saturating_sub(last_seen_at) > self.stale_timeout {
                        HeartbeatFreshness::Stale
                    } else {
                        HeartbeatFreshness::Fresh
                    }
                }
            }
        }

        pub fn observable_status(&self, runtime: RuntimeKey) -> ObservableRuntimeStatus {
            let record = self.record(runtime);
            match record.base_status {
                BaseRuntimeStatus::Stopped => ObservableRuntimeStatus::Stopped,
                BaseRuntimeStatus::Ready => match self.heartbeat_freshness(runtime) {
                    HeartbeatFreshness::Unknown | HeartbeatFreshness::Fresh => {
                        ObservableRuntimeStatus::Ready
                    }
                    HeartbeatFreshness::Stale => ObservableRuntimeStatus::Stale,
                },
            }
        }
    }

    pub fn apply(
        state: &RegistryLivenessState,
        action: RegistryLivenessAction,
    ) -> Option<(RegistryLivenessState, RegistryLivenessTransition)> {
        let mut next = state.clone();
        match action {
            RegistryLivenessAction::Register { runtime } => {
                next.now += 1;
                next.runtimes[runtime.index()] = RuntimeLivenessRecord {
                    base_status: BaseRuntimeStatus::Ready,
                    last_seen_at: Some(next.now),
                };
                Some((
                    next,
                    RegistryLivenessTransition::Registered {
                        runtime,
                        at: state.now + 1,
                    },
                ))
            }
            RegistryLivenessAction::Heartbeat { runtime } => {
                let mut record = next.runtimes[runtime.index()];
                if record.base_status == BaseRuntimeStatus::Stopped {
                    return None;
                }
                next.now += 1;
                record.last_seen_at = Some(next.now);
                next.runtimes[runtime.index()] = record;
                Some((
                    next,
                    RegistryLivenessTransition::HeartbeatRecorded {
                        runtime,
                        at: state.now + 1,
                    },
                ))
            }
            RegistryLivenessAction::AdvanceTime => {
                next.now += 1;
                let now = next.now;
                Some((next, RegistryLivenessTransition::TimeAdvanced { now }))
            }
            RegistryLivenessAction::StaleScan => {
                next.now += 1;
                next.last_scan_at = Some(next.now);
                let at = next.now;
                Some((next, RegistryLivenessTransition::Scanned { at }))
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::{
            apply, HeartbeatFreshness, ObservableRuntimeStatus, RegistryLivenessAction,
            RegistryLivenessState, RuntimeKey,
        };

        #[test]
        fn stale_scan_then_heartbeat_restores_ready_from_single_registry_state() {
            let state = RegistryLivenessState::default();

            let (state, _) = apply(
                &state,
                RegistryLivenessAction::Register {
                    runtime: RuntimeKey::A,
                },
            )
            .expect("register");
            let (state, _) = apply(
                &state,
                RegistryLivenessAction::Heartbeat {
                    runtime: RuntimeKey::A,
                },
            )
            .expect("heartbeat 1");
            let (state, _) = apply(
                &state,
                RegistryLivenessAction::Heartbeat {
                    runtime: RuntimeKey::A,
                },
            )
            .expect("heartbeat 2");
            let (state, _) = apply(&state, RegistryLivenessAction::AdvanceTime).expect("tick 1");
            let (state, _) = apply(&state, RegistryLivenessAction::AdvanceTime).expect("tick 2");
            let (state, _) = apply(&state, RegistryLivenessAction::StaleScan).expect("scan");

            assert_eq!(
                state.heartbeat_freshness(RuntimeKey::A),
                HeartbeatFreshness::Stale
            );
            assert_eq!(
                state.observable_status(RuntimeKey::A),
                ObservableRuntimeStatus::Stale
            );

            let (state, _) = apply(
                &state,
                RegistryLivenessAction::Heartbeat {
                    runtime: RuntimeKey::A,
                },
            )
            .expect("heartbeat after stale");
            assert_eq!(
                state.heartbeat_freshness(RuntimeKey::A),
                HeartbeatFreshness::Fresh
            );
            assert_eq!(
                state.observable_status(RuntimeKey::A),
                ObservableRuntimeStatus::Ready
            );
        }
    }
}

pub mod stream_truth {
    use super::liveness::{BaseRuntimeStatus, RuntimeKey};

    #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
    pub struct RuntimeProjectionRecord {
        pub status: BaseRuntimeStatus,
        pub runtime_id: Option<u64>,
        pub spec_present: bool,
        pub bound_session: bool,
    }

    impl Default for RuntimeProjectionRecord {
        fn default() -> Self {
            Self {
                status: BaseRuntimeStatus::Stopped,
                runtime_id: None,
                spec_present: false,
                bound_session: false,
            }
        }
    }

    #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
    pub enum RuntimeEnvelope {
        RuntimeSpecPersisted {
            runtime: RuntimeKey,
            runtime_id: u64,
            bound_session: bool,
        },
        RuntimeStopped {
            runtime: RuntimeKey,
        },
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq)]
    pub struct StreamTruthState {
        pub log: Vec<RuntimeEnvelope>,
        pub runtime_index: [RuntimeProjectionRecord; 2],
        pub next_runtime_id: u64,
    }

    impl Default for StreamTruthState {
        fn default() -> Self {
            Self {
                log: Vec::new(),
                runtime_index: [
                    RuntimeProjectionRecord::default(),
                    RuntimeProjectionRecord::default(),
                ],
                next_runtime_id: 1,
            }
        }
    }

    #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
    pub enum StreamTruthAction {
        PersistRuntimeSpec { runtime: RuntimeKey },
        StopRuntime { runtime: RuntimeKey },
    }

    #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
    pub enum StreamTruthTransition {
        RuntimeSpecPersisted {
            runtime: RuntimeKey,
            runtime_id: u64,
        },
        RuntimeStopped {
            runtime: RuntimeKey,
        },
    }

    pub fn project_runtime_index(log: &[RuntimeEnvelope]) -> [RuntimeProjectionRecord; 2] {
        let mut projected = [
            RuntimeProjectionRecord::default(),
            RuntimeProjectionRecord::default(),
        ];

        for envelope in log {
            match *envelope {
                RuntimeEnvelope::RuntimeSpecPersisted {
                    runtime,
                    runtime_id,
                    bound_session,
                } => {
                    projected[runtime.index()] = RuntimeProjectionRecord {
                        status: BaseRuntimeStatus::Ready,
                        runtime_id: Some(runtime_id),
                        spec_present: true,
                        bound_session,
                    };
                }
                RuntimeEnvelope::RuntimeStopped { runtime } => {
                    let mut record = projected[runtime.index()];
                    record.status = BaseRuntimeStatus::Stopped;
                    projected[runtime.index()] = record;
                }
            }
        }

        projected
    }

    pub fn apply(
        state: &StreamTruthState,
        action: StreamTruthAction,
    ) -> Option<(StreamTruthState, StreamTruthTransition)> {
        let mut next = state.clone();
        match action {
            StreamTruthAction::PersistRuntimeSpec { runtime } => {
                let runtime_id = next.next_runtime_id;
                next.next_runtime_id += 1;
                next.log.push(RuntimeEnvelope::RuntimeSpecPersisted {
                    runtime,
                    runtime_id,
                    bound_session: true,
                });
                next.runtime_index[runtime.index()] = RuntimeProjectionRecord {
                    status: BaseRuntimeStatus::Ready,
                    runtime_id: Some(runtime_id),
                    spec_present: true,
                    bound_session: true,
                };
                Some((
                    next,
                    StreamTruthTransition::RuntimeSpecPersisted {
                        runtime,
                        runtime_id,
                    },
                ))
            }
            StreamTruthAction::StopRuntime { runtime } => {
                let record = next.runtime_index[runtime.index()];
                if !record.spec_present || record.status == BaseRuntimeStatus::Stopped {
                    return None;
                }
                next.log.push(RuntimeEnvelope::RuntimeStopped { runtime });
                let mut updated = record;
                updated.status = BaseRuntimeStatus::Stopped;
                next.runtime_index[runtime.index()] = updated;
                Some((next, StreamTruthTransition::RuntimeStopped { runtime }))
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::{apply, project_runtime_index, StreamTruthAction, StreamTruthState};
        use crate::liveness::RuntimeKey;

        #[test]
        fn projected_runtime_index_matches_incremental_materialization() {
            let state = StreamTruthState::default();
            let (state, _) = apply(
                &state,
                StreamTruthAction::PersistRuntimeSpec {
                    runtime: RuntimeKey::A,
                },
            )
            .expect("persist A");
            let (state, _) = apply(
                &state,
                StreamTruthAction::PersistRuntimeSpec {
                    runtime: RuntimeKey::B,
                },
            )
            .expect("persist B");
            let (state, _) = apply(
                &state,
                StreamTruthAction::StopRuntime {
                    runtime: RuntimeKey::A,
                },
            )
            .expect("stop A");
            let (state, _) = apply(
                &state,
                StreamTruthAction::PersistRuntimeSpec {
                    runtime: RuntimeKey::A,
                },
            )
            .expect("re-provision A");

            assert_eq!(project_runtime_index(&state.log), state.runtime_index);
        }
    }
}

pub mod session {
    use std::collections::BTreeSet;

    #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
    pub enum ProducerId {
        Harness,
        ApprovalService,
    }

    #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
    pub enum SessionEventId {
        SessionCreated,
        PromptTurnStarted,
        ApprovalResolved,
    }

    #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
    pub enum SessionEventKind {
        SessionCreated,
        PromptTurnStarted,
        ApprovalResolved,
    }

    #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
    pub struct ProducerCommit {
        pub producer_id: ProducerId,
        pub epoch: u64,
        pub seq: u64,
    }

    #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
    pub struct LoggedEvent {
        pub logical_event_id: SessionEventId,
        pub commit: ProducerCommit,
        pub kind: SessionEventKind,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq)]
    pub struct ReplayObservation {
        pub offset: usize,
        pub captured_log: Vec<LoggedEvent>,
        pub suffix: Vec<LoggedEvent>,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq)]
    pub struct SessionState {
        pub log: Vec<LoggedEvent>,
        pub seen_commits: BTreeSet<ProducerCommit>,
        pub runtime_alive: bool,
    }

    impl Default for SessionState {
        fn default() -> Self {
            Self {
                log: Vec::new(),
                seen_commits: BTreeSet::new(),
                runtime_alive: true,
            }
        }
    }

    #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
    pub enum SessionAction {
        Append {
            commit: ProducerCommit,
            logical_event_id: SessionEventId,
            kind: SessionEventKind,
        },
        ReplayFromOffset {
            offset: usize,
        },
        CrashRuntime,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq)]
    pub enum SessionTransition {
        Appended,
        DedupedRetry,
        Replayed(ReplayObservation),
        RuntimeCrashed,
    }

    pub fn replay_suffix(log: &[LoggedEvent], offset: usize) -> Vec<LoggedEvent> {
        if offset >= log.len() {
            Vec::new()
        } else {
            log[offset..].to_vec()
        }
    }

    pub fn apply(
        state: &SessionState,
        action: SessionAction,
    ) -> Option<(SessionState, SessionTransition)> {
        match action {
            SessionAction::Append {
                commit,
                logical_event_id,
                kind,
            } => {
                let mut next = state.clone();
                let transition = if next.seen_commits.insert(commit) {
                    next.log.push(LoggedEvent {
                        logical_event_id,
                        commit,
                        kind,
                    });
                    SessionTransition::Appended
                } else {
                    SessionTransition::DedupedRetry
                };
                Some((next, transition))
            }
            SessionAction::ReplayFromOffset { offset } => {
                if offset > state.log.len() {
                    return None;
                }
                let captured_log = state.log.clone();
                let replay = ReplayObservation {
                    offset,
                    suffix: replay_suffix(&captured_log, offset),
                    captured_log,
                };
                Some((state.clone(), SessionTransition::Replayed(replay)))
            }
            SessionAction::CrashRuntime => {
                if !state.runtime_alive {
                    return None;
                }
                let mut next = state.clone();
                next.runtime_alive = false;
                Some((next, SessionTransition::RuntimeCrashed))
            }
        }
    }
}

pub mod approval {
    #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
    pub enum ApprovalRequestId {
        Expected,
        Noise,
    }

    #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
    pub enum Decision {
        Allow,
        Deny,
    }

    #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
    pub enum ApprovalPhase {
        Idle,
        Blocked,
        Completed,
        Denied,
    }

    #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
    pub struct ApprovalRecord {
        pub request_id: ApprovalRequestId,
        pub decision: Decision,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq)]
    pub struct ApprovalState {
        pub phase: ApprovalPhase,
        pub blocked_request: Option<ApprovalRequestId>,
        pub history: Vec<ApprovalRecord>,
        pub completion_count: u64,
        pub retry_count: u64,
    }

    impl Default for ApprovalState {
        fn default() -> Self {
            Self {
                phase: ApprovalPhase::Idle,
                blocked_request: None,
                history: Vec::new(),
                completion_count: 0,
                retry_count: 0,
            }
        }
    }

    #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
    pub enum ApprovalAction {
        Request {
            request_id: ApprovalRequestId,
        },
        Resolve {
            request_id: ApprovalRequestId,
            decision: Decision,
        },
        RetryBlocked,
        AdvanceBlocked,
    }

    #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
    pub enum ApprovalOutcome {
        Blocked,
        RecordedDecision,
        Retried,
        AdvancedAllowed,
        AdvancedDenied,
        Noop,
    }

    pub fn first_resolution_for(
        state: &ApprovalState,
        request_id: ApprovalRequestId,
    ) -> Option<Decision> {
        state
            .history
            .iter()
            .find_map(|record| (record.request_id == request_id).then_some(record.decision))
    }

    pub fn first_matching_resolution(state: &ApprovalState) -> Option<Decision> {
        let blocked_request = state.blocked_request?;
        first_resolution_for(state, blocked_request)
    }

    pub fn apply(
        state: &ApprovalState,
        action: ApprovalAction,
    ) -> Option<(ApprovalState, ApprovalOutcome)> {
        match action {
            ApprovalAction::Request { request_id } => {
                if state.phase != ApprovalPhase::Idle || state.blocked_request.is_some() {
                    return None;
                }
                let mut next = state.clone();
                next.phase = ApprovalPhase::Blocked;
                next.blocked_request = Some(request_id);
                Some((next, ApprovalOutcome::Blocked))
            }
            ApprovalAction::Resolve {
                request_id,
                decision,
            } => {
                let mut next = state.clone();
                next.history.push(ApprovalRecord {
                    request_id,
                    decision,
                });
                Some((next, ApprovalOutcome::RecordedDecision))
            }
            ApprovalAction::RetryBlocked => {
                if state.phase != ApprovalPhase::Blocked {
                    return None;
                }
                let mut next = state.clone();
                next.retry_count += 1;
                Some((next, ApprovalOutcome::Retried))
            }
            ApprovalAction::AdvanceBlocked => {
                if state.phase != ApprovalPhase::Blocked {
                    return None;
                }
                let Some(first) = first_matching_resolution(state) else {
                    return Some((state.clone(), ApprovalOutcome::Noop));
                };
                let mut next = state.clone();
                next.blocked_request = None;
                match first {
                    Decision::Allow => {
                        if next.completion_count == 0 {
                            next.completion_count += 1;
                        }
                        next.phase = ApprovalPhase::Completed;
                        Some((next, ApprovalOutcome::AdvancedAllowed))
                    }
                    Decision::Deny => {
                        next.phase = ApprovalPhase::Denied;
                        Some((next, ApprovalOutcome::AdvancedDenied))
                    }
                }
            }
        }
    }
}

pub mod resume {
    use std::collections::BTreeSet;

    #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
    pub enum ResumeScenario {
        Live,
        Cold,
    }

    #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
    pub enum RuntimeStatus {
        Ready,
        Starting,
        Stopped,
    }

    #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
    pub enum Caller {
        A,
        B,
    }

    impl Caller {
        pub const ALL: [Caller; 2] = [Caller::A, Caller::B];

        pub const fn index(self) -> usize {
            match self {
                Caller::A => 0,
                Caller::B => 1,
            }
        }
    }

    #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
    pub enum CallerPhase {
        Idle,
        Inspecting,
        NeedsProvision,
        WaitingForReady,
        Done,
    }

    #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
    pub struct CallerState {
        pub phase: CallerPhase,
        pub observed_runtime_id: Option<u64>,
    }

    impl Default for CallerState {
        fn default() -> Self {
            Self {
                phase: CallerPhase::Idle,
                observed_runtime_id: None,
            }
        }
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq)]
    pub struct ResumeState {
        pub scenario: ResumeScenario,
        pub runtime_key: u64,
        pub initial_runtime_id: u64,
        pub active_runtime_id: u64,
        pub pending_runtime_id: Option<u64>,
        pub next_runtime_id: u64,
        pub runtime_status: RuntimeStatus,
        pub callers: [CallerState; 2],
        pub reprovision_count: u64,
        pub session_exists: bool,
        pub persisted_spec: bool,
    }

    impl ResumeState {
        pub fn new(scenario: ResumeScenario) -> Self {
            let runtime_status = match scenario {
                ResumeScenario::Live => RuntimeStatus::Ready,
                ResumeScenario::Cold => RuntimeStatus::Stopped,
            };
            Self {
                scenario,
                runtime_key: 1,
                initial_runtime_id: 1,
                active_runtime_id: 1,
                pending_runtime_id: None,
                next_runtime_id: 2,
                runtime_status,
                callers: [CallerState::default(), CallerState::default()],
                reprovision_count: 0,
                session_exists: true,
                persisted_spec: true,
            }
        }

        pub fn observed_ids(&self) -> BTreeSet<u64> {
            self.callers
                .iter()
                .filter_map(|caller| caller.observed_runtime_id)
                .collect()
        }
    }

    #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
    pub enum ResumeAction {
        Begin(Caller),
        Inspect(Caller),
        CreateOrJoin(Caller),
        RegisterStartedRuntime,
        Finish(Caller),
    }

    #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
    pub enum ResumeOutcome {
        BeganInspecting,
        InspectedLive,
        InspectedNeedsProvision,
        InspectedWaiting,
        ReprovisionStarted(u64),
        JoinedPending,
        RegisteredStartedRuntime(u64),
        Finished(u64),
    }

    pub fn apply(
        state: &ResumeState,
        action: ResumeAction,
    ) -> Option<(ResumeState, ResumeOutcome)> {
        let mut next = state.clone();
        match action {
            ResumeAction::Begin(caller) => {
                let caller_state = &mut next.callers[caller.index()];
                if caller_state.phase != CallerPhase::Idle {
                    return None;
                }
                caller_state.phase = CallerPhase::Inspecting;
                Some((next, ResumeOutcome::BeganInspecting))
            }
            ResumeAction::Inspect(caller) => {
                let caller_state = &mut next.callers[caller.index()];
                if caller_state.phase != CallerPhase::Inspecting {
                    return None;
                }
                match next.runtime_status {
                    RuntimeStatus::Ready => {
                        caller_state.phase = CallerPhase::Done;
                        caller_state.observed_runtime_id = Some(next.active_runtime_id);
                        Some((next, ResumeOutcome::InspectedLive))
                    }
                    RuntimeStatus::Stopped => {
                        caller_state.phase = CallerPhase::NeedsProvision;
                        Some((next, ResumeOutcome::InspectedNeedsProvision))
                    }
                    RuntimeStatus::Starting => {
                        caller_state.phase = CallerPhase::WaitingForReady;
                        Some((next, ResumeOutcome::InspectedWaiting))
                    }
                }
            }
            ResumeAction::CreateOrJoin(caller) => {
                let caller_state = &mut next.callers[caller.index()];
                if caller_state.phase != CallerPhase::NeedsProvision
                    && caller_state.phase != CallerPhase::WaitingForReady
                {
                    return None;
                }
                match next.runtime_status {
                    RuntimeStatus::Stopped => {
                        next.runtime_status = RuntimeStatus::Starting;
                        next.pending_runtime_id = Some(next.next_runtime_id);
                        let created = next.next_runtime_id;
                        next.next_runtime_id += 1;
                        next.reprovision_count += 1;
                        caller_state.phase = CallerPhase::WaitingForReady;
                        Some((next, ResumeOutcome::ReprovisionStarted(created)))
                    }
                    RuntimeStatus::Starting => {
                        caller_state.phase = CallerPhase::WaitingForReady;
                        Some((next, ResumeOutcome::JoinedPending))
                    }
                    RuntimeStatus::Ready => {
                        caller_state.phase = CallerPhase::Done;
                        let runtime_id = next.active_runtime_id;
                        caller_state.observed_runtime_id = Some(runtime_id);
                        Some((next, ResumeOutcome::Finished(runtime_id)))
                    }
                }
            }
            ResumeAction::RegisterStartedRuntime => {
                if next.runtime_status != RuntimeStatus::Starting {
                    return None;
                }
                let pending = next.pending_runtime_id?;
                next.active_runtime_id = pending;
                next.pending_runtime_id = None;
                next.runtime_status = RuntimeStatus::Ready;
                Some((next, ResumeOutcome::RegisteredStartedRuntime(pending)))
            }
            ResumeAction::Finish(caller) => {
                let caller_state = &mut next.callers[caller.index()];
                if caller_state.phase != CallerPhase::WaitingForReady
                    || next.runtime_status != RuntimeStatus::Ready
                {
                    return None;
                }
                caller_state.phase = CallerPhase::Done;
                let runtime_id = next.active_runtime_id;
                caller_state.observed_runtime_id = Some(runtime_id);
                Some((next, ResumeOutcome::Finished(runtime_id)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::approval::{
        apply as apply_approval, first_matching_resolution, ApprovalAction, ApprovalPhase,
        ApprovalRequestId, ApprovalState, Decision,
    };
    use super::resume::{
        apply as apply_resume, Caller, ResumeAction, ResumeOutcome, ResumeScenario, ResumeState,
        RuntimeStatus,
    };
    use super::session::{
        apply as apply_session, ProducerCommit, ProducerId, SessionAction, SessionEventId,
        SessionEventKind, SessionState, SessionTransition,
    };

    #[test]
    fn session_append_dedupes_by_commit_tuple() {
        let state = SessionState::default();
        let commit = ProducerCommit {
            producer_id: ProducerId::Harness,
            epoch: 0,
            seq: 0,
        };

        let (state, transition) = apply_session(
            &state,
            SessionAction::Append {
                commit,
                logical_event_id: SessionEventId::PromptTurnStarted,
                kind: SessionEventKind::PromptTurnStarted,
            },
        )
        .expect("first append valid");
        assert_eq!(transition, SessionTransition::Appended);
        assert_eq!(state.log.len(), 1);

        let (state, transition) = apply_session(
            &state,
            SessionAction::Append {
                commit,
                logical_event_id: SessionEventId::PromptTurnStarted,
                kind: SessionEventKind::PromptTurnStarted,
            },
        )
        .expect("duplicate append is still a valid retry");
        assert_eq!(transition, SessionTransition::DedupedRetry);
        assert_eq!(state.log.len(), 1);
    }

    #[test]
    fn approval_first_matching_resolution_is_stable() {
        let (state, _) = apply_approval(
            &ApprovalState::default(),
            ApprovalAction::Request {
                request_id: ApprovalRequestId::Expected,
            },
        )
        .expect("request valid");
        let (state, _) = apply_approval(
            &state,
            ApprovalAction::Resolve {
                request_id: ApprovalRequestId::Expected,
                decision: Decision::Deny,
            },
        )
        .expect("deny valid");
        let (state, _) = apply_approval(
            &state,
            ApprovalAction::Resolve {
                request_id: ApprovalRequestId::Expected,
                decision: Decision::Allow,
            },
        )
        .expect("late allow still records");

        assert_eq!(first_matching_resolution(&state), Some(Decision::Deny));
        let (state, outcome) =
            apply_approval(&state, ApprovalAction::AdvanceBlocked).expect("advance valid");
        assert_eq!(state.phase, ApprovalPhase::Denied);
        assert_eq!(outcome, super::approval::ApprovalOutcome::AdvancedDenied);
    }

    #[test]
    fn cold_resume_reprovisions_then_registers() {
        let state = ResumeState::new(ResumeScenario::Cold);
        let (state, _) = apply_resume(&state, ResumeAction::Begin(Caller::A)).expect("begin");
        let (state, _) = apply_resume(&state, ResumeAction::Inspect(Caller::A)).expect("inspect");
        let (state, outcome) =
            apply_resume(&state, ResumeAction::CreateOrJoin(Caller::A)).expect("create");
        let created = match outcome {
            ResumeOutcome::ReprovisionStarted(id) => id,
            other => panic!("unexpected outcome: {other:?}"),
        };
        assert_eq!(state.runtime_status, RuntimeStatus::Starting);

        let (state, outcome) =
            apply_resume(&state, ResumeAction::RegisterStartedRuntime).expect("register");
        assert_eq!(outcome, ResumeOutcome::RegisteredStartedRuntime(created));
        assert_eq!(state.runtime_status, RuntimeStatus::Ready);
        assert_eq!(state.active_runtime_id, created);
    }
}
