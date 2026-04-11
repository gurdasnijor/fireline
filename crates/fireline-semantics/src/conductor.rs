use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum EffectKind {
    Init,
    Prompt,
    ToolCall,
    PeerCall,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum ContextTag {
    Injected,
    Template,
    Audit,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum ToolName {
    ListPeers,
    PromptPeer,
    SearchFiles,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum DescriptionId {
    Opaque,
    PeerPrompting,
    Search,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum InputSchemaId {
    Opaque,
    PromptPeerInput,
    SearchInput,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum TransportRef {
    Smithery,
    PeerAcp,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum CredentialRef {
    AgentPw,
    None,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct ToolDescriptor {
    pub name: ToolName,
    pub description: DescriptionId,
    pub input_schema: InputSchemaId,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct CapabilityAttachment {
    pub descriptor: ToolDescriptor,
    pub transport_ref: TransportRef,
    pub credential_ref: CredentialRef,
}

pub fn project_tool_descriptor(attachment: CapabilityAttachment) -> ToolDescriptor {
    attachment.descriptor
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum SourceRef {
    Workspace,
    Repo,
    Artifact,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum MountPath {
    Workdir,
    Repo,
    Cache,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct ResourceRef {
    pub source_ref: SourceRef,
    pub mount_path: MountPath,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct Effect {
    pub kind: EffectKind,
    pub context: Vec<ContextTag>,
    pub tools: Vec<ToolDescriptor>,
    pub mounts: Vec<ResourceRef>,
}

impl Effect {
    pub fn new(kind: EffectKind) -> Self {
        Self {
            kind,
            context: Vec::new(),
            tools: Vec::new(),
            mounts: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum SessionArtifactKind {
    Audit,
    Trace,
    PermissionRequest,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct SessionArtifact {
    pub artifact_kind: SessionArtifactKind,
    pub effect_kind: EffectKind,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum Rejection {
    BudgetExceeded,
    PolicyBlocked,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum SuspendReason {
    ApprovalRequired,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub enum EffectResult {
    Completed { final_effects: Vec<Effect> },
    Rejected(Rejection),
    Suspended(SuspendReason),
    Branched(Vec<EffectResult>),
}

#[derive(Clone, Debug, Default, Hash, PartialEq, Eq)]
pub struct InvocationTrace {
    pub observed_effects: Vec<Effect>,
    pub downstream_calls: Vec<Effect>,
    pub session_appends: Vec<SessionArtifact>,
    pub mount_events: Vec<ResourceRef>,
}

#[derive(Clone, Debug, Default, Hash, PartialEq, Eq)]
pub struct HarnessState {
    pub session_log: Vec<SessionArtifact>,
    pub mounted_resources: BTreeSet<ResourceRef>,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum MapEffectOp {
    AddContext(ContextTag),
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum SessionAppendOp {
    Audit,
    Trace,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum FilterPredicate {
    ToolCallsOnly,
    PeerCallsOnly,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum SubstituteOp {
    PeerCallToToolCall,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum SuspendPredicate {
    ToolCallsRequireApproval,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum FanoutPlan {
    PromptToToolAndPeer,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub enum Component {
    Identity,
    Observe,
    MapEffect(MapEffectOp),
    AppendToSession(SessionAppendOp),
    Filter {
        predicate: FilterPredicate,
        rejection: Rejection,
    },
    Substitute(SubstituteOp),
    Suspend(SuspendPredicate),
    Fanout(FanoutPlan),
    RegisterTool(ToolDescriptor),
    Provision(Vec<ResourceRef>),
}

pub fn compose(left: &[Component], right: &[Component]) -> Vec<Component> {
    let mut combined = Vec::with_capacity(left.len() + right.len());
    combined.extend_from_slice(left);
    combined.extend_from_slice(right);
    combined
}

pub fn invoke(
    state: &HarnessState,
    components: &[Component],
    effect: Effect,
) -> (HarnessState, InvocationTrace, EffectResult) {
    let mut next = state.clone();
    let mut trace = InvocationTrace::default();
    let result = apply_chain(&mut next, &mut trace, components, effect);
    (next, trace, result)
}

fn apply_chain(
    state: &mut HarnessState,
    trace: &mut InvocationTrace,
    components: &[Component],
    effect: Effect,
) -> EffectResult {
    let Some((component, tail)) = components.split_first() else {
        trace.downstream_calls.push(effect.clone());
        return EffectResult::Completed {
            final_effects: vec![effect],
        };
    };

    match component {
        Component::Identity => apply_chain(state, trace, tail, effect),
        Component::Observe => {
            trace.observed_effects.push(effect.clone());
            apply_chain(state, trace, tail, effect)
        }
        Component::MapEffect(op) => {
            let mapped = map_effect(effect, *op);
            apply_chain(state, trace, tail, mapped)
        }
        Component::AppendToSession(op) => {
            append_session_artifact(state, trace, effect.kind, *op);
            apply_chain(state, trace, tail, effect)
        }
        Component::Filter {
            predicate,
            rejection,
        } => {
            if matches_filter(effect.kind, *predicate) {
                EffectResult::Rejected(*rejection)
            } else {
                apply_chain(state, trace, tail, effect)
            }
        }
        Component::Substitute(op) => {
            let rewritten = substitute_effect(effect, *op);
            apply_chain(state, trace, tail, rewritten)
        }
        Component::Suspend(predicate) => {
            if matches_suspend(effect.kind, *predicate) {
                let artifact = SessionArtifact {
                    artifact_kind: SessionArtifactKind::PermissionRequest,
                    effect_kind: effect.kind,
                };
                state.session_log.push(artifact.clone());
                trace.session_appends.push(artifact);
                EffectResult::Suspended(SuspendReason::ApprovalRequired)
            } else {
                apply_chain(state, trace, tail, effect)
            }
        }
        Component::Fanout(plan) => {
            let split = split_effect(effect, *plan);
            if split.len() == 1 {
                apply_chain(
                    state,
                    trace,
                    tail,
                    split.into_iter().next().expect("one effect"),
                )
            } else {
                EffectResult::Branched(
                    split
                        .into_iter()
                        .map(|branch| apply_chain(state, trace, tail, branch))
                        .collect(),
                )
            }
        }
        Component::RegisterTool(tool) => {
            if effect.kind == EffectKind::Init {
                let mut next_effect = effect;
                push_unique(&mut next_effect.tools, *tool);
                apply_chain(state, trace, tail, next_effect)
            } else {
                apply_chain(state, trace, tail, effect)
            }
        }
        Component::Provision(resources) => {
            if effect.kind == EffectKind::Init {
                let mut next_effect = effect;
                for resource in resources {
                    if state.mounted_resources.insert(*resource) {
                        trace.mount_events.push(*resource);
                    }
                    push_unique(&mut next_effect.mounts, *resource);
                }
                apply_chain(state, trace, tail, next_effect)
            } else {
                apply_chain(state, trace, tail, effect)
            }
        }
    }
}

fn map_effect(mut effect: Effect, op: MapEffectOp) -> Effect {
    match op {
        MapEffectOp::AddContext(tag) => {
            effect.context.push(tag);
            effect
        }
    }
}

fn substitute_effect(mut effect: Effect, op: SubstituteOp) -> Effect {
    match op {
        SubstituteOp::PeerCallToToolCall => {
            if effect.kind == EffectKind::PeerCall {
                effect.kind = EffectKind::ToolCall;
            }
            effect
        }
    }
}

fn split_effect(effect: Effect, plan: FanoutPlan) -> Vec<Effect> {
    match plan {
        FanoutPlan::PromptToToolAndPeer if effect.kind == EffectKind::Prompt => {
            let mut tool_branch = effect.clone();
            tool_branch.kind = EffectKind::ToolCall;
            let mut peer_branch = effect;
            peer_branch.kind = EffectKind::PeerCall;
            vec![tool_branch, peer_branch]
        }
        _ => vec![effect],
    }
}

fn matches_filter(kind: EffectKind, predicate: FilterPredicate) -> bool {
    match predicate {
        FilterPredicate::ToolCallsOnly => kind == EffectKind::ToolCall,
        FilterPredicate::PeerCallsOnly => kind == EffectKind::PeerCall,
    }
}

fn matches_suspend(kind: EffectKind, predicate: SuspendPredicate) -> bool {
    match predicate {
        SuspendPredicate::ToolCallsRequireApproval => kind == EffectKind::ToolCall,
    }
}

fn append_session_artifact(
    state: &mut HarnessState,
    trace: &mut InvocationTrace,
    effect_kind: EffectKind,
    op: SessionAppendOp,
) {
    let artifact = SessionArtifact {
        artifact_kind: match op {
            SessionAppendOp::Audit => SessionArtifactKind::Audit,
            SessionAppendOp::Trace => SessionArtifactKind::Trace,
        },
        effect_kind,
    };
    state.session_log.push(artifact.clone());
    trace.session_appends.push(artifact);
}

fn push_unique<T>(items: &mut Vec<T>, item: T)
where
    T: PartialEq,
{
    if !items.contains(&item) {
        items.push(item);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn list_peers_tool() -> ToolDescriptor {
        ToolDescriptor {
            name: ToolName::ListPeers,
            description: DescriptionId::Opaque,
            input_schema: InputSchemaId::Opaque,
        }
    }

    fn workspace_resource() -> ResourceRef {
        ResourceRef {
            source_ref: SourceRef::Workspace,
            mount_path: MountPath::Workdir,
        }
    }

    fn arb_effect() -> impl Strategy<Value = Effect> {
        prop_oneof![
            Just(Effect::new(EffectKind::Init)),
            Just(Effect::new(EffectKind::Prompt)),
            Just(Effect::new(EffectKind::ToolCall)),
            Just(Effect::new(EffectKind::PeerCall)),
        ]
    }

    fn arb_component_atom() -> impl Strategy<Value = Component> {
        prop_oneof![
            Just(Component::Identity),
            Just(Component::Observe),
            Just(Component::MapEffect(MapEffectOp::AddContext(
                ContextTag::Injected
            ))),
            Just(Component::MapEffect(MapEffectOp::AddContext(
                ContextTag::Template
            ))),
            Just(Component::AppendToSession(SessionAppendOp::Audit)),
            Just(Component::AppendToSession(SessionAppendOp::Trace)),
            Just(Component::Filter {
                predicate: FilterPredicate::ToolCallsOnly,
                rejection: Rejection::BudgetExceeded,
            }),
            Just(Component::Substitute(SubstituteOp::PeerCallToToolCall)),
            Just(Component::RegisterTool(list_peers_tool())),
            Just(Component::Provision(vec![workspace_resource()])),
        ]
    }

    fn arb_component_list() -> impl Strategy<Value = Vec<Component>> {
        prop::collection::vec(arb_component_atom(), 0..6)
    }

    proptest! {
        #[test]
        fn compose_left_identity_holds(components in arb_component_list(), effect in arb_effect()) {
            let state = HarnessState::default();
            let identity = vec![Component::Identity];
            let left = compose(&identity, &components);
            let direct = invoke(&state, &components, effect.clone());
            let through_identity = invoke(&state, &left, effect);
            prop_assert_eq!(through_identity, direct);
        }

        #[test]
        fn compose_right_identity_holds(components in arb_component_list(), effect in arb_effect()) {
            let state = HarnessState::default();
            let identity = vec![Component::Identity];
            let right = compose(&components, &identity);
            let direct = invoke(&state, &components, effect.clone());
            let through_identity = invoke(&state, &right, effect);
            prop_assert_eq!(through_identity, direct);
        }

        #[test]
        fn compose_associativity_holds(
            a in arb_component_list(),
            b in arb_component_list(),
            c in arb_component_list(),
            effect in arb_effect(),
        ) {
            let state = HarnessState::default();
            let left = compose(&a, &compose(&b, &c));
            let right = compose(&compose(&a, &b), &c);
            let left_result = invoke(&state, &left, effect.clone());
            let right_result = invoke(&state, &right, effect);
            prop_assert_eq!(left_result, right_result);
        }
    }

    #[test]
    fn map_effect_composition_accumulates_context_in_order() {
        let state = HarnessState::default();
        let effect = Effect::new(EffectKind::Prompt);
        let components = vec![
            Component::MapEffect(MapEffectOp::AddContext(ContextTag::Injected)),
            Component::MapEffect(MapEffectOp::AddContext(ContextTag::Template)),
        ];

        let (_, trace, result) = invoke(&state, &components, effect);
        assert!(trace.session_appends.is_empty());
        assert_eq!(
            result,
            EffectResult::Completed {
                final_effects: vec![Effect {
                    kind: EffectKind::Prompt,
                    context: vec![ContextTag::Injected, ContextTag::Template],
                    tools: Vec::new(),
                    mounts: Vec::new(),
                }],
            }
        );
    }

    #[test]
    fn append_to_session_preserves_downstream_result() {
        let state = HarnessState::default();
        let effect = Effect::new(EffectKind::Prompt);
        let (_, _, direct) = invoke(&state, &[], effect.clone());
        let (next, trace, with_append) = invoke(
            &state,
            &[Component::AppendToSession(SessionAppendOp::Trace)],
            effect,
        );

        assert_eq!(with_append, direct);
        assert_eq!(trace.session_appends.len(), 1);
        assert_eq!(next.session_log.len(), 1);
    }

    #[test]
    fn filter_does_not_delegate_rejected_effects() {
        let state = HarnessState::default();
        let effect = Effect::new(EffectKind::ToolCall);
        let (_, trace, result) = invoke(
            &state,
            &[Component::Filter {
                predicate: FilterPredicate::ToolCallsOnly,
                rejection: Rejection::PolicyBlocked,
            }],
            effect,
        );

        assert_eq!(result, EffectResult::Rejected(Rejection::PolicyBlocked));
        assert!(trace.downstream_calls.is_empty());
    }

    #[test]
    fn register_tool_is_init_only() {
        let state = HarnessState::default();
        let tool = list_peers_tool();

        let (_, _, init_result) = invoke(
            &state,
            &[Component::RegisterTool(tool)],
            Effect::new(EffectKind::Init),
        );
        assert_eq!(
            init_result,
            EffectResult::Completed {
                final_effects: vec![Effect {
                    kind: EffectKind::Init,
                    context: Vec::new(),
                    tools: vec![tool],
                    mounts: Vec::new(),
                }],
            }
        );

        let (_, _, prompt_result) = invoke(
            &state,
            &[Component::RegisterTool(tool)],
            Effect::new(EffectKind::Prompt),
        );
        assert_eq!(
            prompt_result,
            EffectResult::Completed {
                final_effects: vec![Effect::new(EffectKind::Prompt)],
            }
        );
    }

    #[test]
    fn provision_is_single_fire_per_runtime_state() {
        let state = HarnessState::default();
        let components = vec![Component::Provision(vec![workspace_resource()])];

        let (state, trace, _) = invoke(&state, &components, Effect::new(EffectKind::Init));
        assert_eq!(trace.mount_events, vec![workspace_resource()]);
        assert_eq!(state.mounted_resources.len(), 1);

        let (state, trace, _) = invoke(&state, &components, Effect::new(EffectKind::Init));
        assert!(trace.mount_events.is_empty());
        assert_eq!(state.mounted_resources.len(), 1);

        let (_, trace, result) = invoke(&state, &components, Effect::new(EffectKind::Prompt));
        assert!(trace.mount_events.is_empty());
        assert_eq!(
            result,
            EffectResult::Completed {
                final_effects: vec![Effect::new(EffectKind::Prompt)],
            }
        );
    }

    #[test]
    fn suspend_logs_permission_request_and_blocks_progress() {
        let state = HarnessState::default();
        let effect = Effect::new(EffectKind::ToolCall);
        let (state, trace, result) = invoke(
            &state,
            &[Component::Suspend(
                SuspendPredicate::ToolCallsRequireApproval,
            )],
            effect,
        );

        assert_eq!(
            result,
            EffectResult::Suspended(SuspendReason::ApprovalRequired)
        );
        assert!(trace.downstream_calls.is_empty());
        assert_eq!(
            state.session_log,
            vec![SessionArtifact {
                artifact_kind: SessionArtifactKind::PermissionRequest,
                effect_kind: EffectKind::ToolCall,
            }]
        );
    }

    #[test]
    fn tool_projection_is_schema_only() {
        let descriptor = ToolDescriptor {
            name: ToolName::PromptPeer,
            description: DescriptionId::PeerPrompting,
            input_schema: InputSchemaId::PromptPeerInput,
        };
        let capability = CapabilityAttachment {
            descriptor,
            transport_ref: TransportRef::PeerAcp,
            credential_ref: CredentialRef::AgentPw,
        };

        assert_eq!(project_tool_descriptor(capability), descriptor);
    }

    #[test]
    fn tool_projection_is_transport_agnostic() {
        let descriptor = ToolDescriptor {
            name: ToolName::SearchFiles,
            description: DescriptionId::Search,
            input_schema: InputSchemaId::SearchInput,
        };
        let a = CapabilityAttachment {
            descriptor,
            transport_ref: TransportRef::Smithery,
            credential_ref: CredentialRef::AgentPw,
        };
        let b = CapabilityAttachment {
            descriptor,
            transport_ref: TransportRef::PeerAcp,
            credential_ref: CredentialRef::None,
        };

        assert_eq!(project_tool_descriptor(a), project_tool_descriptor(b));
    }

    #[test]
    fn fanout_splits_prompt_into_two_branches() {
        let state = HarnessState::default();
        let (_, trace, result) = invoke(
            &state,
            &[Component::Fanout(FanoutPlan::PromptToToolAndPeer)],
            Effect::new(EffectKind::Prompt),
        );

        assert_eq!(
            result,
            EffectResult::Branched(vec![
                EffectResult::Completed {
                    final_effects: vec![Effect::new(EffectKind::ToolCall)],
                },
                EffectResult::Completed {
                    final_effects: vec![Effect::new(EffectKind::PeerCall)],
                },
            ])
        );
        assert_eq!(trace.downstream_calls.len(), 2);
    }
}
