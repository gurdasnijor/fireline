#![cfg_attr(not(test), allow(dead_code))]

//! Stateright protocol models for Fireline's managed-agent substrate.
//!
//! These models are intentionally small. They do not execute production code
//! and they do not attempt whole-program verification. They model the race and
//! retry seams that the managed-agent contract suite already names explicitly:
//!
//! - session append/replay/dedupe
//! - live and cold `resume(session_id)` behavior
//! - approval-gate release under retries and duplicate resolutions
//!
//! The refinement mapping that ties these models back to the executable tests
//! lives at `verification/docs/refinement-matrix.md`.

use std::collections::BTreeSet;

use fireline_semantics::{
    approval::{
        apply as apply_approval, first_resolution_for, ApprovalAction as ApprovalProtocolAction,
        ApprovalPhase as PromptPhase, ApprovalRequestId as RequestId,
        ApprovalState as ApprovalProtocolState, Decision as ApprovalResolution,
    },
    resume::{
        apply as apply_resume, Caller, CallerPhase, ResumeAction as ResumeProtocolAction,
        ResumeScenario, ResumeState as ResumeProtocolState, RuntimeStatus,
    },
    session::{
        apply as apply_session, replay_suffix, LoggedEvent, ProducerCommit, ProducerId,
        ReplayObservation, SessionAction, SessionEventId, SessionEventKind,
        SessionState as SemanticSessionState, SessionTransition,
    },
};
use stateright::{Model, Property};

fn is_prefix<T: PartialEq>(prefix: &[T], full: &[T]) -> bool {
    prefix.len() <= full.len() && prefix.iter().zip(full.iter()).all(|(a, b)| a == b)
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct SessionProtocolState {
    core: SemanticSessionState,
    snapshots: Vec<Vec<LoggedEvent>>,
    last_replay: Option<ReplayObservation>,
    death_prefix: Option<Vec<LoggedEvent>>,
}

impl Default for SessionProtocolState {
    fn default() -> Self {
        Self {
            core: SemanticSessionState::default(),
            snapshots: vec![Vec::new()],
            last_replay: None,
            death_prefix: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
enum SessionProtocolAction {
    AppendSessionCreated,
    AppendPromptTurn,
    RetryPromptTurnSameScope,
    ReplayFromStart,
    ReplayFromOne,
    CrashRuntime,
    ExternalApprovalAfterCrash,
}

#[derive(Clone, Default)]
struct SessionProtocolModel;

impl SessionProtocolModel {
    const fn session_created_commit() -> ProducerCommit {
        ProducerCommit {
            producer_id: ProducerId::Harness,
            epoch: 0,
            seq: 0,
        }
    }

    const fn prompt_turn_commit() -> ProducerCommit {
        ProducerCommit {
            producer_id: ProducerId::Harness,
            epoch: 0,
            seq: 1,
        }
    }

    const fn approval_after_crash_commit() -> ProducerCommit {
        ProducerCommit {
            producer_id: ProducerId::ApprovalService,
            epoch: 0,
            seq: 0,
        }
    }

    fn commit_tuples_unique(state: &SessionProtocolState) -> bool {
        state
            .core
            .log
            .iter()
            .map(|logged| logged.commit)
            .collect::<BTreeSet<_>>()
            .len()
            == state.core.log.len()
    }

    fn snapshots_are_prefix_monotonic(state: &SessionProtocolState) -> bool {
        state
            .snapshots
            .windows(2)
            .all(|pair| is_prefix(&pair[0], &pair[1]))
    }
}

impl Model for SessionProtocolModel {
    type State = SessionProtocolState;
    type Action = SessionProtocolAction;

    fn init_states(&self) -> Vec<Self::State> {
        vec![SessionProtocolState::default()]
    }

    fn actions(&self, state: &Self::State, actions: &mut Vec<Self::Action>) {
        if !state
            .core
            .seen_commits
            .contains(&Self::session_created_commit())
        {
            actions.push(SessionProtocolAction::AppendSessionCreated);
        }
        if state.core.runtime_alive
            && !state
                .core
                .seen_commits
                .contains(&Self::prompt_turn_commit())
        {
            actions.push(SessionProtocolAction::AppendPromptTurn);
        }
        if state
            .core
            .seen_commits
            .contains(&Self::prompt_turn_commit())
        {
            actions.push(SessionProtocolAction::RetryPromptTurnSameScope);
        }
        actions.push(SessionProtocolAction::ReplayFromStart);
        if !state.core.log.is_empty() {
            actions.push(SessionProtocolAction::ReplayFromOne);
        }
        if state.core.runtime_alive {
            actions.push(SessionProtocolAction::CrashRuntime);
        }
        if !state.core.runtime_alive
            && !state
                .core
                .seen_commits
                .contains(&Self::approval_after_crash_commit())
        {
            actions.push(SessionProtocolAction::ExternalApprovalAfterCrash);
        }
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        match action {
            SessionProtocolAction::AppendSessionCreated => {
                let (core, _) = apply_session(
                    &state.core,
                    SessionAction::Append {
                        commit: Self::session_created_commit(),
                        logical_event_id: SessionEventId::SessionCreated,
                        kind: SessionEventKind::SessionCreated,
                    },
                )?;
                let mut next = state.clone();
                next.core = core;
                next.snapshots.push(next.core.log.clone());
                Some(next)
            }
            SessionProtocolAction::AppendPromptTurn => {
                let (core, _) = apply_session(
                    &state.core,
                    SessionAction::Append {
                        commit: Self::prompt_turn_commit(),
                        logical_event_id: SessionEventId::PromptTurnStarted,
                        kind: SessionEventKind::PromptTurnStarted,
                    },
                )?;
                let mut next = state.clone();
                next.core = core;
                next.snapshots.push(next.core.log.clone());
                Some(next)
            }
            SessionProtocolAction::RetryPromptTurnSameScope => {
                let (core, transition) = apply_session(
                    &state.core,
                    SessionAction::Append {
                        commit: Self::prompt_turn_commit(),
                        logical_event_id: SessionEventId::PromptTurnStarted,
                        kind: SessionEventKind::PromptTurnStarted,
                    },
                )?;
                debug_assert!(matches!(transition, SessionTransition::DedupedRetry));
                let mut next = state.clone();
                next.core = core;
                next.snapshots.push(next.core.log.clone());
                Some(next)
            }
            SessionProtocolAction::ReplayFromStart => {
                let (_, transition) =
                    apply_session(&state.core, SessionAction::ReplayFromOffset { offset: 0 })?;
                let mut next = state.clone();
                if let SessionTransition::Replayed(replay) = transition {
                    next.last_replay = Some(replay);
                }
                next.snapshots.push(next.core.log.clone());
                Some(next)
            }
            SessionProtocolAction::ReplayFromOne => {
                let (_, transition) =
                    apply_session(&state.core, SessionAction::ReplayFromOffset { offset: 1 })?;
                let mut next = state.clone();
                if let SessionTransition::Replayed(replay) = transition {
                    next.last_replay = Some(replay);
                }
                next.snapshots.push(next.core.log.clone());
                Some(next)
            }
            SessionProtocolAction::CrashRuntime => {
                let (core, _) = apply_session(&state.core, SessionAction::CrashRuntime)?;
                let mut next = state.clone();
                next.core = core;
                if next.death_prefix.is_none() {
                    next.death_prefix = Some(next.core.log.clone());
                }
                next.snapshots.push(next.core.log.clone());
                Some(next)
            }
            SessionProtocolAction::ExternalApprovalAfterCrash => {
                let (core, _) = apply_session(
                    &state.core,
                    SessionAction::Append {
                        commit: Self::approval_after_crash_commit(),
                        logical_event_id: SessionEventId::ApprovalResolved,
                        kind: SessionEventKind::ApprovalResolved,
                    },
                )?;
                let mut next = state.clone();
                next.core = core;
                next.snapshots.push(next.core.log.clone());
                Some(next)
            }
        }
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always("SessionAppendOnly", |_, state| {
                Self::snapshots_are_prefix_monotonic(state)
            }),
            Property::always(
                "SessionReplayFromOffsetIsSuffix",
                |_, state: &SessionProtocolState| {
                    state
                        .last_replay
                        .as_ref()
                        .is_none_or(|obs| obs.suffix == replay_suffix(&obs.captured_log, obs.offset))
                },
            ),
            Property::always(
                "SessionDurableAcrossRuntimeDeath",
                |_, state: &SessionProtocolState| {
                    state
                        .death_prefix
                        .as_ref()
                        .is_none_or(|prefix| is_prefix(prefix, &state.core.log))
                },
            ),
            Property::always(
                "SessionScopedIdempotentAppend",
                |_, state: &SessionProtocolState| Self::commit_tuples_unique(state),
            ),
            Property::sometimes(
                "SessionCrashStillAllowsExternalAppend",
                |_, state: &SessionProtocolState| {
                    !state.core.runtime_alive
                        && state.core.log.iter().any(|event| {
                            event.commit.producer_id == ProducerId::ApprovalService
                                && event.kind == SessionEventKind::ApprovalResolved
                        })
                },
            ),
        ]
    }

    fn within_boundary(&self, state: &Self::State) -> bool {
        state.core.log.len() <= 3 && state.snapshots.len() <= 8
    }
}

#[derive(Clone)]
struct ResumeProtocolModel {
    scenario: ResumeScenario,
}

impl ResumeProtocolModel {
    fn live() -> Self {
        Self {
            scenario: ResumeScenario::Live,
        }
    }

    fn cold() -> Self {
        Self {
            scenario: ResumeScenario::Cold,
        }
    }
}

impl Model for ResumeProtocolModel {
    type State = ResumeProtocolState;
    type Action = ResumeProtocolAction;

    fn init_states(&self) -> Vec<Self::State> {
        vec![ResumeProtocolState::new(self.scenario)]
    }

    fn actions(&self, state: &Self::State, actions: &mut Vec<Self::Action>) {
        for caller in Caller::ALL {
            match state.callers[caller.index()].phase {
                CallerPhase::Idle => actions.push(ResumeProtocolAction::Begin(caller)),
                CallerPhase::Inspecting => actions.push(ResumeProtocolAction::Inspect(caller)),
                CallerPhase::NeedsProvision => {
                    actions.push(ResumeProtocolAction::CreateOrJoin(caller));
                }
                CallerPhase::WaitingForReady => {
                    if state.runtime_status == RuntimeStatus::Ready {
                        actions.push(ResumeProtocolAction::Finish(caller));
                    } else if state.runtime_status == RuntimeStatus::Stopped {
                        actions.push(ResumeProtocolAction::CreateOrJoin(caller));
                    }
                }
                CallerPhase::Done => {}
            }
        }
        if state.runtime_status == RuntimeStatus::Starting {
            actions.push(ResumeProtocolAction::RegisterStartedRuntime);
        }
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        apply_resume(state, action).map(|(next, _)| next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always(
                "ResumeOnLiveRuntimeIsNoop",
                |_, state: &ResumeProtocolState| {
                    if state.scenario != ResumeScenario::Live {
                        return true;
                    }
                    state.reprovision_count == 0
                        && state.callers.iter().all(|caller| {
                            caller
                                .observed_runtime_id
                                .is_none_or(|id| id == state.initial_runtime_id)
                        })
                },
            ),
            Property::always(
                "ConcurrentResumeSingleWinner",
                |_, state: &ResumeProtocolState| {
                    let unique_ids = state.observed_ids();
                    unique_ids.len() <= 1 && state.reprovision_count <= 1
                },
            ),
            Property::always(
                "ColdResumePreservesRuntimeKeyChangesRuntimeId",
                |_, state: &ResumeProtocolState| {
                    if state.scenario != ResumeScenario::Cold
                        || !state
                            .callers
                            .iter()
                            .any(|caller| caller.phase == CallerPhase::Done)
                    {
                        return true;
                    }
                    state.runtime_key == 1
                        && state.runtime_status == RuntimeStatus::Ready
                        && state.active_runtime_id != state.initial_runtime_id
                },
            ),
            Property::sometimes(
                "ResumeCompletesForAllCallers",
                |_, state: &ResumeProtocolState| {
                    state.runtime_status == RuntimeStatus::Ready
                        && state
                            .callers
                            .iter()
                            .all(|caller| caller.phase == CallerPhase::Done)
                },
            ),
        ]
    }

    fn within_boundary(&self, state: &Self::State) -> bool {
        state.reprovision_count <= 1
            && state.next_runtime_id <= 3
            && state.session_exists
            && state.persisted_spec
    }
}

#[derive(Clone, Default)]
struct ApprovalProtocolModel;

impl ApprovalProtocolModel {
    fn request_emitted(state: &ApprovalProtocolState) -> bool {
        state.phase != PromptPhase::Idle || !state.history.is_empty()
    }
}

impl Model for ApprovalProtocolModel {
    type State = ApprovalProtocolState;
    type Action = ApprovalProtocolAction;

    fn init_states(&self) -> Vec<Self::State> {
        vec![ApprovalProtocolState::default()]
    }

    fn actions(&self, state: &Self::State, actions: &mut Vec<Self::Action>) {
        if state.phase == PromptPhase::Idle {
            actions.push(ApprovalProtocolAction::Request {
                request_id: RequestId::Expected,
            });
        }
        if Self::request_emitted(state) {
            actions.push(ApprovalProtocolAction::Resolve {
                request_id: RequestId::Expected,
                decision: ApprovalResolution::Allow,
            });
            actions.push(ApprovalProtocolAction::Resolve {
                request_id: RequestId::Expected,
                decision: ApprovalResolution::Deny,
            });
            actions.push(ApprovalProtocolAction::Resolve {
                request_id: RequestId::Noise,
                decision: ApprovalResolution::Allow,
            });
            actions.push(ApprovalProtocolAction::Resolve {
                request_id: RequestId::Noise,
                decision: ApprovalResolution::Deny,
            });
        }
        if state.phase == PromptPhase::Blocked {
            if state.retry_count < 2 {
                actions.push(ApprovalProtocolAction::RetryBlocked);
            }
            actions.push(ApprovalProtocolAction::AdvanceBlocked);
        }
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        apply_approval(state, action).map(|(next, _)| next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always(
                "HarnessSuspendReleasedOnlyByMatchingApproval",
                |_, state: &ApprovalProtocolState| {
                    !matches!(state.phase, PromptPhase::Completed)
                        || first_resolution_for(state, RequestId::Expected)
                            == Some(ApprovalResolution::Allow)
                },
            ),
            Property::always(
                "ApprovalDuplicateResolutionDoesNotDuplicateProgress",
                |_, state: &ApprovalProtocolState| state.completion_count <= 1,
            ),
            Property::always(
                "BlockedRequestDoesNotAdvanceBeforeApproval",
                |_, state: &ApprovalProtocolState| {
                    state.completion_count == 0
                        || first_resolution_for(state, RequestId::Expected)
                            == Some(ApprovalResolution::Allow)
                },
            ),
            Property::always(
                "ApprovalTerminalDecisionFollowsFirstMatchingResolution",
                |_, state: &ApprovalProtocolState| match state.phase {
                    PromptPhase::Completed => {
                        first_resolution_for(state, RequestId::Expected)
                            == Some(ApprovalResolution::Allow)
                    }
                    PromptPhase::Denied => {
                        first_resolution_for(state, RequestId::Expected)
                            == Some(ApprovalResolution::Deny)
                    }
                    _ => true,
                },
            ),
            Property::sometimes(
                "ApprovalRaceEventuallyTerminates",
                |_, state: &ApprovalProtocolState| {
                    matches!(state.phase, PromptPhase::Completed | PromptPhase::Denied)
                },
            ),
        ]
    }

    fn within_boundary(&self, state: &Self::State) -> bool {
        state.retry_count <= 2 && state.history.len() <= 4
    }
}

#[cfg(test)]
mod tests {
    use stateright::{Checker, Model};

    use super::{ApprovalProtocolModel, ResumeProtocolModel, SessionProtocolModel};

    #[test]
    fn session_protocol_model_checks_core_session_invariants() {
        let checker = SessionProtocolModel::default().checker().spawn_bfs().join();
        checker.assert_properties();
    }

    #[test]
    fn live_resume_model_checks_noop_and_single_winner_properties() {
        let checker = ResumeProtocolModel::live().checker().spawn_bfs().join();
        checker.assert_properties();
    }

    #[test]
    fn cold_resume_model_checks_reprovision_properties() {
        let checker = ResumeProtocolModel::cold().checker().spawn_bfs().join();
        checker.assert_properties();
    }

    #[test]
    fn approval_protocol_model_checks_release_race_properties() {
        let checker = ApprovalProtocolModel::default()
            .checker()
            .spawn_bfs()
            .join();
        checker.assert_properties();
    }
}
