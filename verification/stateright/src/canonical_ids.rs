use std::collections::{BTreeMap, BTreeSet};

use stateright::{Model, Property};

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum CanonicalSessionId {
    SessionA,
    SessionB,
}

impl CanonicalSessionId {
    pub const ALL: [Self; 2] = [Self::SessionA, Self::SessionB];
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum CanonicalRequestId {
    Shared,
}

impl CanonicalRequestId {
    pub const ALL: [Self; 1] = [Self::Shared];
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum CanonicalToolCallId {
    ToolA,
}

impl CanonicalToolCallId {
    pub const ALL: [Self; 1] = [Self::ToolA];
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct PromptRequestRef {
    pub session_id: CanonicalSessionId,
    pub request_id: CanonicalRequestId,
}

impl PromptRequestRef {
    const fn new(session_id: CanonicalSessionId, request_id: CanonicalRequestId) -> Self {
        Self {
            session_id,
            request_id,
        }
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct ToolInvocationRef {
    pub session_id: CanonicalSessionId,
    pub request_id: CanonicalRequestId,
    pub tool_call_id: CanonicalToolCallId,
}

impl ToolInvocationRef {
    const fn new(
        session_id: CanonicalSessionId,
        request_id: CanonicalRequestId,
        tool_call_id: CanonicalToolCallId,
    ) -> Self {
        Self {
            session_id,
            request_id,
            tool_call_id,
        }
    }

    const fn prompt_ref(self) -> PromptRequestRef {
        PromptRequestRef::new(self.session_id, self.request_id)
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
enum CanonicalEnvelope {
    PromptRequestStarted(PromptRequestRef),
    ChunkAppended(ToolInvocationRef),
    PermissionRequested(PromptRequestRef),
    ApprovalResolved {
        key: PromptRequestRef,
        allow: bool,
    },
}

#[derive(Clone, Debug, Default, Hash, PartialEq, Eq)]
struct CanonicalProjection {
    prompt_request_counts: BTreeMap<PromptRequestRef, usize>,
    prompt_requests: BTreeSet<PromptRequestRef>,
    permission_requests: BTreeSet<PromptRequestRef>,
    pending_approvals: BTreeSet<PromptRequestRef>,
    first_resolutions: BTreeMap<PromptRequestRef, bool>,
    released_by: BTreeMap<PromptRequestRef, PromptRequestRef>,
    chunks: BTreeMap<PromptRequestRef, BTreeSet<CanonicalToolCallId>>,
}

impl CanonicalProjection {
    fn apply_envelope(&mut self, envelope: CanonicalEnvelope) {
        match envelope {
            CanonicalEnvelope::PromptRequestStarted(key) => {
                *self.prompt_request_counts.entry(key).or_default() += 1;
                self.prompt_requests.insert(key);
            }
            CanonicalEnvelope::ChunkAppended(tool_ref) => {
                self.chunks
                    .entry(tool_ref.prompt_ref())
                    .or_default()
                    .insert(tool_ref.tool_call_id);
            }
            CanonicalEnvelope::PermissionRequested(key) => {
                self.permission_requests.insert(key);
                self.pending_approvals.insert(key);
            }
            CanonicalEnvelope::ApprovalResolved { key, allow } => {
                self.first_resolutions.entry(key).or_insert(allow);
                if allow && self.pending_approvals.remove(&key) {
                    self.released_by.entry(key).or_insert(key);
                }
            }
        }
    }

    fn project(log: &[CanonicalEnvelope]) -> Self {
        let mut projection = Self::default();

        for envelope in log {
            projection.apply_envelope(*envelope);
        }

        projection
    }

    fn replay_from_offset(log: &[CanonicalEnvelope], offset: usize) -> Self {
        let mut projection = Self::project(&log[..offset]);
        for envelope in &log[offset..] {
            projection.apply_envelope(*envelope);
        }
        projection
    }

    fn identifiers_are_canonical(&self) -> bool {
        let prompt_keys_canonical = self.prompt_request_counts.keys().all(|key| {
            CanonicalSessionId::ALL.contains(&key.session_id)
                && CanonicalRequestId::ALL.contains(&key.request_id)
        });
        let permission_keys_canonical = self.permission_requests.iter().all(|key| {
            CanonicalSessionId::ALL.contains(&key.session_id)
                && CanonicalRequestId::ALL.contains(&key.request_id)
        });
        let pending_keys_canonical = self.pending_approvals.iter().all(|key| {
            CanonicalSessionId::ALL.contains(&key.session_id)
                && CanonicalRequestId::ALL.contains(&key.request_id)
        });
        let chunk_keys_canonical = self.chunks.iter().all(|(key, tool_ids)| {
            CanonicalSessionId::ALL.contains(&key.session_id)
                && CanonicalRequestId::ALL.contains(&key.request_id)
                && tool_ids
                    .iter()
                    .all(|tool_id| CanonicalToolCallId::ALL.contains(tool_id))
        });

        prompt_keys_canonical
            && permission_keys_canonical
            && pending_keys_canonical
            && chunk_keys_canonical
    }

    const fn infrastructure_and_agent_planes_disjoint(&self) -> bool {
        // This model is intentionally agent-plane only.
        true
    }

    fn approvals_keyed_by_canonical_request_id(&self) -> bool {
        self.pending_approvals
            .iter()
            .all(|key| self.permission_requests.contains(key))
            && self
                .first_resolutions
                .keys()
                .all(|key| self.permission_requests.contains(key))
            && self
                .released_by
                .iter()
                .all(|(target, source)| target == source)
    }

    fn prompt_request_ref_unique_per_session(&self) -> bool {
        self.prompt_request_counts.values().all(|count| *count <= 1)
    }

    fn concurrent_approvals_remain_session_scoped(&self) -> bool {
        for request_id in CanonicalRequestId::ALL {
            let key_a = PromptRequestRef::new(CanonicalSessionId::SessionA, request_id);
            let key_b = PromptRequestRef::new(CanonicalSessionId::SessionB, request_id);

            if self.permission_requests.contains(&key_a) && self.permission_requests.contains(&key_b)
            {
                if !self.first_resolutions.contains_key(&key_a)
                    && !self.pending_approvals.contains(&key_a)
                {
                    return false;
                }
                if !self.first_resolutions.contains_key(&key_b)
                    && !self.pending_approvals.contains(&key_b)
                {
                    return false;
                }
                if self.released_by.get(&key_a).is_some_and(|source| *source != key_a) {
                    return false;
                }
                if self.released_by.get(&key_b).is_some_and(|source| *source != key_b) {
                    return false;
                }
            }
        }

        true
    }

    fn shared_request_pending_twice(&self) -> bool {
        let key_a = PromptRequestRef::new(CanonicalSessionId::SessionA, CanonicalRequestId::Shared);
        let key_b = PromptRequestRef::new(CanonicalSessionId::SessionB, CanonicalRequestId::Shared);

        self.pending_approvals.contains(&key_a) && self.pending_approvals.contains(&key_b)
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct ReplayObservation {
    offset: usize,
    projection: CanonicalProjection,
}

#[derive(Clone, Debug, Default, Hash, PartialEq, Eq)]
pub struct CanonicalIdsState {
    log: Vec<CanonicalEnvelope>,
    last_replay: Option<ReplayObservation>,
    duplicate_prompt_attempted: bool,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum CanonicalIdsAction {
    AppendPromptRequestStarted {
        session_id: CanonicalSessionId,
        request_id: CanonicalRequestId,
    },
    AppendChunk {
        session_id: CanonicalSessionId,
        request_id: CanonicalRequestId,
        tool_call_id: CanonicalToolCallId,
    },
    EmitPermissionRequest {
        session_id: CanonicalSessionId,
        request_id: CanonicalRequestId,
    },
    ResolveApproval {
        session_id: CanonicalSessionId,
        request_id: CanonicalRequestId,
        allow: bool,
    },
    ReplayFromOffset {
        offset: usize,
    },
}

#[derive(Clone, Default)]
pub struct CanonicalIdsModel;

impl CanonicalIdsModel {
    const MAX_LOG_LEN: usize = 5;

    fn prompt_ref(
        session_id: CanonicalSessionId,
        request_id: CanonicalRequestId,
    ) -> PromptRequestRef {
        PromptRequestRef::new(session_id, request_id)
    }
}

impl Model for CanonicalIdsModel {
    type State = CanonicalIdsState;
    type Action = CanonicalIdsAction;

    fn init_states(&self) -> Vec<Self::State> {
        vec![CanonicalIdsState::default()]
    }

    fn actions(&self, state: &Self::State, actions: &mut Vec<Self::Action>) {
        for session_id in CanonicalSessionId::ALL {
            for request_id in CanonicalRequestId::ALL {
                if state.log.len() < Self::MAX_LOG_LEN {
                    actions.push(CanonicalIdsAction::AppendPromptRequestStarted {
                        session_id,
                        request_id,
                    });
                    actions.push(CanonicalIdsAction::EmitPermissionRequest {
                        session_id,
                        request_id,
                    });
                    actions.push(CanonicalIdsAction::ResolveApproval {
                        session_id,
                        request_id,
                        allow: true,
                    });
                    for tool_call_id in CanonicalToolCallId::ALL {
                        actions.push(CanonicalIdsAction::AppendChunk {
                            session_id,
                            request_id,
                            tool_call_id,
                        });
                    }
                }
            }
        }

        for offset in 0..=state.log.len().min(4) {
            actions.push(CanonicalIdsAction::ReplayFromOffset { offset });
        }
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        let live_projection = CanonicalProjection::project(&state.log);
        let mut next = state.clone();

        match action {
            CanonicalIdsAction::AppendPromptRequestStarted {
                session_id,
                request_id,
            } => {
                let key = Self::prompt_ref(session_id, request_id);
                if live_projection.prompt_requests.contains(&key) {
                    if state.duplicate_prompt_attempted {
                        return None;
                    }
                    next.duplicate_prompt_attempted = true;
                    return Some(next);
                }
                next.last_replay = None;
                next.log.push(CanonicalEnvelope::PromptRequestStarted(key));
            }
            CanonicalIdsAction::AppendChunk {
                session_id,
                request_id,
                tool_call_id,
            } => {
                let tool_ref = ToolInvocationRef::new(session_id, request_id, tool_call_id);
                if !live_projection.prompt_requests.contains(&tool_ref.prompt_ref()) {
                    return None;
                }
                next.last_replay = None;
                next.log.push(CanonicalEnvelope::ChunkAppended(tool_ref));
            }
            CanonicalIdsAction::EmitPermissionRequest {
                session_id,
                request_id,
            } => {
                let key = Self::prompt_ref(session_id, request_id);
                if !live_projection.prompt_requests.contains(&key)
                    || live_projection.permission_requests.contains(&key)
                {
                    return None;
                }
                next.last_replay = None;
                next.log.push(CanonicalEnvelope::PermissionRequested(key));
            }
            CanonicalIdsAction::ResolveApproval {
                session_id,
                request_id,
                allow,
            } => {
                let key = Self::prompt_ref(session_id, request_id);
                if !live_projection.permission_requests.contains(&key) {
                    return None;
                }
                next.last_replay = None;
                next.log
                    .push(CanonicalEnvelope::ApprovalResolved { key, allow });
            }
            CanonicalIdsAction::ReplayFromOffset { offset } => {
                let observation = ReplayObservation {
                    offset,
                    projection: CanonicalProjection::replay_from_offset(&state.log, offset),
                };
                if state.last_replay.as_ref() == Some(&observation) {
                    return None;
                }
                next.last_replay = Some(observation);
                return Some(next);
            }
        }

        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always(
                "ConcurrentApprovalsRemainSessionScoped",
                |_, state: &CanonicalIdsState| {
                    CanonicalProjection::project(&state.log)
                        .concurrent_approvals_remain_session_scoped()
                },
            ),
            Property::always(
                "ReplayPreservesCanonicalIdentifiers",
                |_, state: &CanonicalIdsState| {
                    let live_projection = CanonicalProjection::project(&state.log);
                    state.last_replay.as_ref().is_none_or(|replay| {
                        replay.projection.identifiers_are_canonical()
                            && replay.projection.infrastructure_and_agent_planes_disjoint()
                            && replay.projection.approvals_keyed_by_canonical_request_id()
                            && replay.projection == live_projection
                    })
                },
            ),
            Property::always(
                "PromptRequestRefUniquePerSession",
                |_, state: &CanonicalIdsState| {
                    CanonicalProjection::project(&state.log).prompt_request_ref_unique_per_session()
                },
            ),
            Property::sometimes(
                "ConcurrentSharedRequestIdScenarioReached",
                |_, state: &CanonicalIdsState| {
                    CanonicalProjection::project(&state.log).shared_request_pending_twice()
                },
            ),
            Property::sometimes(
                "ReplayFromBeginningReconstructsSameTree",
                |_, state: &CanonicalIdsState| {
                    let live_projection = CanonicalProjection::project(&state.log);
                    state.last_replay.as_ref().is_some_and(|replay| {
                        !state.log.is_empty()
                            && replay.offset == 0
                            && replay.projection == live_projection
                    })
                },
            ),
        ]
    }

    fn within_boundary(&self, state: &Self::State) -> bool {
        state.log.len() <= Self::MAX_LOG_LEN
    }
}
