use stateright::{Model, Property};

#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
enum WaitState {
    #[default]
    Idle,
    Waiting,
    Completed,
}

#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
struct RegistrationSnapshot {
    state: WaitState,
    completion_count: u8,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
enum RegistrationId {
    First,
    Second,
}

impl RegistrationId {
    const ALL: [Self; 2] = [Self::First, Self::Second];

    const fn index(self) -> usize {
        match self {
            Self::First => 0,
            Self::Second => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
enum LogEvent {
    Register { registration: RegistrationId },
    Complete,
}

#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
struct Projection {
    registrations: [RegistrationSnapshot; 2],
    winner_count: u8,
}

impl Projection {
    fn apply(self, event: LogEvent) -> Self {
        match event {
            LogEvent::Register { registration } => self.register(registration),
            LogEvent::Complete => self.complete(),
        }
    }

    fn register(mut self, registration: RegistrationId) -> Self {
        let slot = &mut self.registrations[registration.index()];
        if slot.state != WaitState::Idle {
            return self;
        }
        if self.winner_count > 0 {
            slot.state = WaitState::Completed;
            slot.completion_count = 1;
        } else {
            slot.state = WaitState::Waiting;
        }
        self
    }

    fn complete(mut self) -> Self {
        if self.winner_count == 0 {
            self.winner_count = 1;
            for registration in &mut self.registrations {
                if registration.state != WaitState::Idle {
                    registration.state = WaitState::Completed;
                    registration.completion_count = 1;
                }
            }
        }
        self
    }
}

#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
struct RebuildSnapshot {
    prefix_len: usize,
    projection: Projection,
}

#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
struct ReplaySnapshot {
    log_len: usize,
    projection: Projection,
}

#[derive(Clone, Debug, Default, Hash, PartialEq, Eq)]
pub(crate) struct DurableSubscriberState {
    log: Vec<LogEvent>,
    live: Projection,
    rebuild: Option<RebuildSnapshot>,
    last_replay: Option<ReplaySnapshot>,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(crate) enum DurableSubscriberAction {
    RegisterFirst,
    RegisterSecond,
    AppendCompletion,
    AppendDuplicateCompletion,
    BeginRebuild,
    CatchUpRebuild,
}

#[derive(Clone, Default)]
pub(crate) struct DurableSubscriberModel;

impl DurableSubscriberModel {
    fn projection_from_log(log: &[LogEvent]) -> Projection {
        log.iter()
            .copied()
            .fold(Projection::default(), Projection::apply)
    }

    fn same_key_waiters_complete_together(state: &DurableSubscriberState) -> bool {
        if state.live.winner_count == 0 {
            return true;
        }
        state
            .live
            .registrations
            .iter()
            .filter(|registration| registration.state != WaitState::Idle)
            .all(|registration| {
                registration.state == WaitState::Completed && registration.completion_count == 1
            })
    }

    fn registrations_complete_at_most_once(state: &DurableSubscriberState) -> bool {
        state
            .live
            .registrations
            .iter()
            .all(|registration| registration.completion_count <= 1)
    }

    fn replay_matches_live(state: &DurableSubscriberState) -> bool {
        state
            .last_replay
            .as_ref()
            .is_none_or(|snapshot| {
                snapshot.log_len != state.log.len() || snapshot.projection == state.live
            })
    }
}

impl Model for DurableSubscriberModel {
    type State = DurableSubscriberState;
    type Action = DurableSubscriberAction;

    fn init_states(&self) -> Vec<Self::State> {
        vec![DurableSubscriberState::default()]
    }

    fn actions(&self, state: &Self::State, actions: &mut Vec<Self::Action>) {
        if state.live.registrations[RegistrationId::First.index()].state == WaitState::Idle {
            actions.push(DurableSubscriberAction::RegisterFirst);
        }
        if state.live.registrations[RegistrationId::Second.index()].state == WaitState::Idle {
            actions.push(DurableSubscriberAction::RegisterSecond);
        }
        if state.live.winner_count == 0 {
            actions.push(DurableSubscriberAction::AppendCompletion);
        } else if state.log.len() < 6 {
            actions.push(DurableSubscriberAction::AppendDuplicateCompletion);
        }

        if state.rebuild.is_none() && !state.log.is_empty() {
            actions.push(DurableSubscriberAction::BeginRebuild);
        }
        if state
            .rebuild
            .is_some_and(|snapshot| snapshot.prefix_len < state.log.len())
        {
            actions.push(DurableSubscriberAction::CatchUpRebuild);
        }
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        let mut next = state.clone();
        match action {
            DurableSubscriberAction::RegisterFirst => {
                next.log.push(LogEvent::Register {
                    registration: RegistrationId::First,
                });
                next.live = Self::projection_from_log(&next.log);
                Some(next)
            }
            DurableSubscriberAction::RegisterSecond => {
                next.log.push(LogEvent::Register {
                    registration: RegistrationId::Second,
                });
                next.live = Self::projection_from_log(&next.log);
                Some(next)
            }
            DurableSubscriberAction::AppendCompletion => {
                next.log.push(LogEvent::Complete);
                next.live = Self::projection_from_log(&next.log);
                Some(next)
            }
            DurableSubscriberAction::AppendDuplicateCompletion => {
                next.log.push(LogEvent::Complete);
                next.live = Self::projection_from_log(&next.log);
                Some(next)
            }
            DurableSubscriberAction::BeginRebuild => {
                next.rebuild = Some(RebuildSnapshot {
                    prefix_len: next.log.len(),
                    projection: Self::projection_from_log(&next.log),
                });
                Some(next)
            }
            DurableSubscriberAction::CatchUpRebuild => {
                let mut rebuild = next.rebuild?;
                for event in next.log[rebuild.prefix_len..].iter().copied() {
                    rebuild.projection = rebuild.projection.apply(event);
                }
                rebuild.prefix_len = next.log.len();
                next.last_replay = Some(ReplaySnapshot {
                    log_len: next.log.len(),
                    projection: rebuild.projection,
                });
                next.rebuild = None;
                Some(next)
            }
        }
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always("FirstResolutionWins", |_, state: &DurableSubscriberState| {
                state.live.winner_count <= 1
                    && Self::registrations_complete_at_most_once(state)
                    && Self::same_key_waiters_complete_together(state)
            }),
            Property::always("RebuildRaceConverges", |_, state: &DurableSubscriberState| {
                Self::replay_matches_live(state)
                    && state.live.winner_count <= 1
                    && Self::registrations_complete_at_most_once(state)
            }),
            Property::sometimes(
                "SharedWaitersEventuallyReachCompletedState",
                |_, state: &DurableSubscriberState| {
                    RegistrationId::ALL.into_iter().all(|registration| {
                        state.live.registrations[registration.index()].state == WaitState::Completed
                    }) && state.live.winner_count == 1
                },
            ),
        ]
    }

    fn within_boundary(&self, state: &Self::State) -> bool {
        state.log.len() <= 6
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
enum SessionId {
    First,
    Second,
}

impl SessionId {
    const ALL: [Self; 2] = [Self::First, Self::Second];

    const fn index(self) -> usize {
        match self {
            Self::First => 0,
            Self::Second => 1,
        }
    }
}

#[derive(Clone, Debug, Default, Hash, PartialEq, Eq)]
struct SessionScopedApprovalState {
    registrations: [RegistrationSnapshot; 2],
    semantic_winners: [u8; 2],
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
enum SessionScopedApprovalAction {
    Register { session: SessionId },
    Resolve { session: SessionId },
    ResolveDuplicate { session: SessionId },
}

#[derive(Clone, Default)]
struct SessionScopedApprovalModel;

impl SessionScopedApprovalModel {
    fn register(mut state: SessionScopedApprovalState, session: SessionId) -> SessionScopedApprovalState {
        let index = session.index();
        let slot = &mut state.registrations[index];
        if slot.state != WaitState::Idle {
            return state;
        }
        if state.semantic_winners[index] > 0 {
            slot.state = WaitState::Completed;
            slot.completion_count = 1;
        } else {
            slot.state = WaitState::Waiting;
        }
        state
    }

    fn resolve(mut state: SessionScopedApprovalState, session: SessionId) -> SessionScopedApprovalState {
        let index = session.index();
        if state.semantic_winners[index] == 0 {
            state.semantic_winners[index] = 1;
            let slot = &mut state.registrations[index];
            if slot.state != WaitState::Idle {
                slot.state = WaitState::Completed;
                slot.completion_count = 1;
            }
        }
        state
    }

    fn session_isolation_holds(state: &SessionScopedApprovalState) -> bool {
        SessionId::ALL.into_iter().all(|session| {
            let index = session.index();
            let registration = state.registrations[index];
            let own_winner = state.semantic_winners[index];

            let self_consistent =
                (registration.state != WaitState::Completed || own_winner == 1)
                    && registration.completion_count <= 1;

            let other_sessions_blocked = SessionId::ALL.into_iter().all(|other| {
                if other == session {
                    return true;
                }
                let other_index = other.index();
                if own_winner == 1 && state.semantic_winners[other_index] == 0 {
                    state.registrations[other_index].state != WaitState::Completed
                        && state.registrations[other_index].completion_count == 0
                } else {
                    true
                }
            });

            self_consistent && other_sessions_blocked
        })
    }
}

impl Model for SessionScopedApprovalModel {
    type State = SessionScopedApprovalState;
    type Action = SessionScopedApprovalAction;

    fn init_states(&self) -> Vec<Self::State> {
        vec![SessionScopedApprovalState::default()]
    }

    fn actions(&self, state: &Self::State, actions: &mut Vec<Self::Action>) {
        for session in SessionId::ALL {
            let index = session.index();
            if state.registrations[index].state == WaitState::Idle {
                actions.push(SessionScopedApprovalAction::Register { session });
            }
            if state.semantic_winners[index] == 0 {
                actions.push(SessionScopedApprovalAction::Resolve { session });
            } else {
                actions.push(SessionScopedApprovalAction::ResolveDuplicate { session });
            }
        }
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        Some(match action {
            SessionScopedApprovalAction::Register { session } => Self::register(state.clone(), session),
            SessionScopedApprovalAction::Resolve { session }
            | SessionScopedApprovalAction::ResolveDuplicate { session } => {
                Self::resolve(state.clone(), session)
            }
        })
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always(
                "OverlappingRequestIdsRemainSessionScoped",
                |_, state: &SessionScopedApprovalState| Self::session_isolation_holds(state),
            ),
            Property::sometimes(
                "OverlappingRequestIdsCanCompleteIndependently",
                |_, state: &SessionScopedApprovalState| {
                    SessionId::ALL.into_iter().all(|session| {
                        let index = session.index();
                        state.semantic_winners[index] == 1
                            && state.registrations[index].state == WaitState::Completed
                    })
                },
            ),
        ]
    }

    fn within_boundary(&self, _state: &Self::State) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use stateright::{Checker, Model};

    use super::{DurableSubscriberModel, SessionScopedApprovalModel};

    #[test]
    fn durable_subscriber_model_first_resolution_wins() {
        let checker = DurableSubscriberModel::default()
            .checker()
            .spawn_bfs()
            .join();
        checker.assert_properties();
    }

    #[test]
    fn durable_subscriber_model_rebuild_race_converges() {
        let checker = DurableSubscriberModel::default()
            .checker()
            .spawn_bfs()
            .join();
        checker.assert_properties();
    }

    #[test]
    fn durable_subscriber_model_overlapping_request_ids_remain_session_scoped() {
        let checker = SessionScopedApprovalModel::default()
            .checker()
            .spawn_bfs()
            .join();
        checker.assert_properties();
    }
}
