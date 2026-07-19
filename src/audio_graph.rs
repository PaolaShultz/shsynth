//! Versioned, bounded model for the SHR-owned stereo audio graph.
//!
//! Validation is control-thread work. Only a validated topological plan may
//! be handed to a real-time callback.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const GRAPH_FORMAT_VERSION: u32 = 1;
pub const EFFECT_FORMAT_VERSION: u32 = 1;
pub const MAX_SOURCES: usize = 4;
pub const MAX_AUX_BUSES: usize = 2;
pub const MAX_CHAIN_EFFECTS: usize = 8;
pub const MAX_EFFECTS: usize = 16;
pub const MAX_NODES: usize = 32;
pub const MAX_EDGES: usize = 64;
pub const MAX_REVERBS: usize = 2;
pub const MAX_CALLBACK_FRAMES: u32 = 4_096;
pub const MAX_EFFECT_MEMORY_BYTES: usize = 16 * 1024 * 1024;

pub type NodeId = u32;
pub type EdgeId = u32;
pub type EffectId = u32;
pub type AuxId = u8;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChannelLayout {
    Mono,
    Stereo,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StereoPorts {
    pub left: String,
    pub right: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SourceKind {
    ManagedEngine,
    LoopPlayer,
    LiveInput { ports: StereoPorts },
    HardwareReturn { loop_id: String, ports: StereoPorts },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SinkKind {
    MainPlayback { ports: StereoPorts },
    HardwareSend { loop_id: String, ports: StereoPorts },
    RecordPreFx,
    RecordPostFx,
    RecordMaster,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum NodeKind {
    Source { source: SourceKind },
    Processor { effect_id: EffectId },
    StereoMixer,
    SendTap { aux_id: AuxId },
    AuxReturn { aux_id: AuxId },
    MonoToStereo,
    Sink { sink: SinkKind },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Node {
    pub id: NodeId,
    pub layout: ChannelLayout,
    pub kind: NodeKind,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Edge {
    pub id: EdgeId,
    pub from: NodeId,
    pub to: NodeId,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EffectKind {
    Utility,
    Eq,
    Compressor,
    Distortion,
    Delay,
    Reverb,
    Chorus,
    Flanger,
    Phaser,
    TremoloPan,
    Filter,
    Gate,
    Crusher,
}

impl EffectKind {
    fn requires_wet_aux(self) -> bool {
        matches!(
            self,
            Self::Delay | Self::Reverb | Self::Chorus | Self::Flanger | Self::Phaser
        )
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EffectInstance {
    pub id: EffectId,
    pub kind: EffectKind,
    pub version: u32,
    pub bypass: bool,
    #[serde(default)]
    pub parameters: BTreeMap<String, f32>,
    #[serde(default)]
    pub owned_memory_bytes: usize,
}

/// Project-owned, ordered inserts for the single managed instrument source.
/// JACK boundary names and runtime node IDs are deliberately not persisted.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InsertRack {
    #[serde(default)]
    pub effects: Vec<EffectInstance>,
    #[serde(default)]
    pub order: Vec<EffectId>,
}

impl InsertRack {
    pub fn validate(&self) -> Result<(), GraphError> {
        if self.effects.len() > MAX_CHAIN_EFFECTS || self.order.len() > MAX_CHAIN_EFFECTS {
            return Err(GraphError::new("serial effect chain bound exceeded"));
        }
        let mut effects = BTreeMap::new();
        for effect in &self.effects {
            if effect.id == 0 || effects.insert(effect.id, effect).is_some() {
                return Err(GraphError::new("effect IDs must be unique and non-zero"));
            }
            if effect.version != EFFECT_FORMAT_VERSION {
                return Err(GraphError::new("unsupported effect version"));
            }
            if !is_insert_effect(effect.kind) {
                return Err(GraphError::new(
                    "effect kind is not available in the insert rack",
                ));
            }
            crate::effect_schema::validate(effect)
                .map_err(|error| GraphError::new(error.to_string()))?;
        }
        if effects.len() != self.order.len() {
            return Err(GraphError::new("rack effects and order must match exactly"));
        }
        let mut ordered = BTreeSet::new();
        for id in &self.order {
            if !effects.contains_key(id) || !ordered.insert(*id) {
                return Err(GraphError::new(
                    "rack order contains a missing or duplicate effect",
                ));
            }
        }
        Ok(())
    }

    pub fn add(&mut self, kind: EffectKind) -> Result<EffectId, GraphError> {
        if !is_insert_effect(kind) {
            return Err(GraphError::new(
                "effect kind is not available in the insert rack",
            ));
        }
        if self.effects.len() >= MAX_CHAIN_EFFECTS {
            return Err(GraphError::new("serial effect chain bound exceeded"));
        }
        let id = self
            .effects
            .iter()
            .map(|effect| effect.id)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .filter(|id| *id != 0)
            .ok_or_else(|| GraphError::new("effect ID space exhausted"))?;
        self.effects.push(EffectInstance {
            id,
            kind,
            version: EFFECT_FORMAT_VERSION,
            bypass: false,
            parameters: crate::effect_schema::defaults(kind),
            owned_memory_bytes: 0,
        });
        self.order.push(id);
        Ok(id)
    }

    pub fn remove(&mut self, id: EffectId) -> Result<EffectInstance, GraphError> {
        let effect_index = self
            .effects
            .iter()
            .position(|effect| effect.id == id)
            .ok_or_else(|| GraphError::new("effect is not in the rack"))?;
        let order_index = self
            .order
            .iter()
            .position(|ordered| *ordered == id)
            .ok_or_else(|| GraphError::new("effect is not in the rack order"))?;
        self.order.remove(order_index);
        Ok(self.effects.remove(effect_index))
    }

    pub fn move_to(&mut self, id: EffectId, index: usize) -> Result<(), GraphError> {
        if index >= self.order.len() {
            return Err(GraphError::new("rack destination is outside the chain"));
        }
        let current = self
            .order
            .iter()
            .position(|ordered| *ordered == id)
            .ok_or_else(|| GraphError::new("effect is not in the rack order"))?;
        let id = self.order.remove(current);
        self.order.insert(index, id);
        Ok(())
    }

    pub fn effect(&self, id: EffectId) -> Option<&EffectInstance> {
        self.effects.iter().find(|effect| effect.id == id)
    }

    pub fn effect_mut(&mut self, id: EffectId) -> Option<&mut EffectInstance> {
        self.effects.iter_mut().find(|effect| effect.id == id)
    }
}

pub const fn is_insert_effect(kind: EffectKind) -> bool {
    matches!(
        kind,
        EffectKind::Utility
            | EffectKind::Eq
            | EffectKind::Compressor
            | EffectKind::Distortion
            | EffectKind::Delay
            | EffectKind::Chorus
            | EffectKind::Flanger
            | EffectKind::Phaser
            | EffectKind::TremoloPan
            | EffectKind::Filter
            | EffectKind::Gate
            | EffectKind::Crusher
    )
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SourceChain {
    pub source_node: NodeId,
    #[serde(default)]
    pub effects: Vec<EffectId>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AuxBus {
    pub id: AuxId,
    #[serde(default)]
    pub effects: Vec<EffectId>,
    pub return_gain_db: f32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SendPoint {
    PreInsert,
    PostInsert,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SendRoute {
    pub source_node: NodeId,
    pub aux_id: AuxId,
    pub level_db: f32,
    pub point: SendPoint,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Monitoring {
    #[serde(default)]
    pub direct: bool,
    #[serde(default)]
    pub software: bool,
    #[serde(default)]
    pub doubled_path_confirmed: bool,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RecordingTap {
    #[default]
    RawInput,
    SourcePreFx,
    SourcePostFx,
    AuxReturn,
    PostMaster,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GraphDefinition {
    pub format_version: u32,
    #[serde(default)]
    pub enabled: bool,
    pub sample_rate: u32,
    pub maximum_callback_frames: u32,
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub edges: Vec<Edge>,
    #[serde(default)]
    pub effects: Vec<EffectInstance>,
    #[serde(default)]
    pub source_chains: Vec<SourceChain>,
    #[serde(default)]
    pub master_chain: Vec<EffectId>,
    #[serde(default)]
    pub aux_buses: Vec<AuxBus>,
    #[serde(default)]
    pub sends: Vec<SendRoute>,
    #[serde(default)]
    pub monitoring: Monitoring,
    #[serde(default)]
    pub recording_tap: RecordingTap,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphError(String);

impl GraphError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for GraphError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for GraphError {}

impl GraphDefinition {
    pub fn validate(&self) -> Result<Vec<NodeId>, GraphError> {
        self.validate_versions_and_bounds()?;
        let nodes = self.validate_nodes()?;
        let effects = self.validate_effects()?;
        self.validate_chains(&nodes, &effects)?;
        self.validate_aux_and_sends(&nodes, &effects)?;
        self.validate_ports_and_monitoring()?;
        let order = self.topological_order(&nodes)?;
        self.validate_hardware_cycles(&nodes, &order)?;
        Ok(order)
    }

    fn validate_versions_and_bounds(&self) -> Result<(), GraphError> {
        if self.format_version != GRAPH_FORMAT_VERSION {
            return Err(GraphError::new(format!(
                "unsupported audio graph version {}",
                self.format_version
            )));
        }
        if !(8_000..=384_000).contains(&self.sample_rate) {
            return Err(GraphError::new("unsupported graph sample rate"));
        }
        if self.maximum_callback_frames == 0 || self.maximum_callback_frames > MAX_CALLBACK_FRAMES {
            return Err(GraphError::new("callback frame bound exceeded"));
        }
        for (count, maximum, label) in [
            (self.nodes.len(), MAX_NODES, "node"),
            (self.edges.len(), MAX_EDGES, "edge"),
            (self.effects.len(), MAX_EFFECTS, "effect"),
            (self.aux_buses.len(), MAX_AUX_BUSES, "aux"),
        ] {
            if count > maximum {
                return Err(GraphError::new(format!("{label} bound exceeded")));
            }
        }
        Ok(())
    }

    fn validate_nodes(&self) -> Result<BTreeMap<NodeId, &Node>, GraphError> {
        let mut nodes = BTreeMap::new();
        let mut sources = 0;
        let mut main_playback = 0;
        let mut aux_returns = BTreeSet::new();
        for node in &self.nodes {
            if node.id == 0 || nodes.insert(node.id, node).is_some() {
                return Err(GraphError::new("node IDs must be unique and non-zero"));
            }
            match &node.kind {
                NodeKind::Source { .. } => sources += 1,
                NodeKind::Sink {
                    sink: SinkKind::MainPlayback { .. },
                } => main_playback += 1,
                NodeKind::Processor { .. }
                | NodeKind::StereoMixer
                | NodeKind::SendTap { .. }
                | NodeKind::AuxReturn { .. }
                | NodeKind::Sink { .. }
                    if node.layout != ChannelLayout::Stereo =>
                {
                    return Err(GraphError::new("stereo node has incompatible layout"));
                }
                NodeKind::MonoToStereo if node.layout != ChannelLayout::Stereo => {
                    return Err(GraphError::new("mono adapter output must be stereo"));
                }
                _ => {}
            }
            if let NodeKind::AuxReturn { aux_id } = node.kind {
                if !aux_returns.insert(aux_id) {
                    return Err(GraphError::new("an aux return may be mixed only once"));
                }
            }
        }
        if sources > MAX_SOURCES {
            return Err(GraphError::new("source bound exceeded"));
        }
        if main_playback > 1 {
            return Err(GraphError::new("main playback sink must be unique"));
        }
        Ok(nodes)
    }

    fn validate_effects(&self) -> Result<BTreeMap<EffectId, &EffectInstance>, GraphError> {
        let mut effects = BTreeMap::new();
        let mut reverbs = 0;
        let mut memory = 0usize;
        for effect in &self.effects {
            if effect.id == 0 || effects.insert(effect.id, effect).is_some() {
                return Err(GraphError::new("effect IDs must be unique and non-zero"));
            }
            if effect.version != EFFECT_FORMAT_VERSION {
                return Err(GraphError::new("unsupported effect version"));
            }
            crate::effect_schema::validate(effect)
                .map_err(|error| GraphError::new(error.to_string()))?;
            let required_memory = crate::effect_schema::minimum_runtime_memory_bytes(
                effect.kind,
                self.sample_rate,
                self.maximum_callback_frames as usize,
            )
            .max(effect.owned_memory_bytes);
            memory = memory
                .checked_add(required_memory)
                .ok_or_else(|| GraphError::new("effect memory overflow"))?;
            reverbs += usize::from(effect.kind == EffectKind::Reverb);
        }
        if memory > MAX_EFFECT_MEMORY_BYTES {
            return Err(GraphError::new("effect memory bound exceeded"));
        }
        if reverbs > MAX_REVERBS {
            return Err(GraphError::new("reverb instance bound exceeded"));
        }
        Ok(effects)
    }

    fn validate_chains(
        &self,
        nodes: &BTreeMap<NodeId, &Node>,
        effects: &BTreeMap<EffectId, &EffectInstance>,
    ) -> Result<(), GraphError> {
        let mut used = BTreeSet::new();
        let mut chained_sources = BTreeSet::new();
        for chain in &self.source_chains {
            if !matches!(
                nodes.get(&chain.source_node).map(|node| &node.kind),
                Some(NodeKind::Source { .. })
            ) {
                return Err(GraphError::new("source chain refers to a missing source"));
            }
            if !chained_sources.insert(chain.source_node) {
                return Err(GraphError::new("source chain is duplicated"));
            }
            validate_chain(&chain.effects, effects, &mut used)?;
        }
        validate_chain(&self.master_chain, effects, &mut used)?;
        for node in self.nodes.iter().filter_map(|node| match node.kind {
            NodeKind::Processor { effect_id } => Some(effect_id),
            _ => None,
        }) {
            if !effects.contains_key(&node) {
                return Err(GraphError::new("processor refers to a missing effect"));
            }
        }
        Ok(())
    }

    fn validate_aux_and_sends(
        &self,
        nodes: &BTreeMap<NodeId, &Node>,
        effects: &BTreeMap<EffectId, &EffectInstance>,
    ) -> Result<(), GraphError> {
        let mut ids = BTreeSet::new();
        let mut used = self
            .source_chains
            .iter()
            .flat_map(|chain| chain.effects.iter().copied())
            .chain(self.master_chain.iter().copied())
            .collect::<BTreeSet<_>>();
        for aux in &self.aux_buses {
            if aux.id == 0 || !ids.insert(aux.id) {
                return Err(GraphError::new("aux IDs must be unique and non-zero"));
            }
            if !aux.return_gain_db.is_finite() || !(-60.0..=12.0).contains(&aux.return_gain_db) {
                return Err(GraphError::new("invalid aux return gain"));
            }
            validate_chain(&aux.effects, effects, &mut used)?;
            for effect_id in &aux.effects {
                let effect = effects[effect_id];
                if effect.kind.requires_wet_aux()
                    && effect.parameters.get("dry_percent").copied() != Some(0.0)
                {
                    return Err(GraphError::new(
                        "aux time/modulation effects must be 100% wet",
                    ));
                }
            }
        }
        let mut send_routes = BTreeSet::new();
        for send in &self.sends {
            if !matches!(
                nodes.get(&send.source_node).map(|node| &node.kind),
                Some(NodeKind::Source { .. })
            ) || !ids.contains(&send.aux_id)
                || !send.level_db.is_finite()
                || !(-60.0..=12.0).contains(&send.level_db)
            {
                return Err(GraphError::new("invalid aux send"));
            }
            if !send_routes.insert((send.source_node, send.aux_id)) {
                return Err(GraphError::new("aux send is duplicated"));
            }
        }
        Ok(())
    }

    fn validate_ports_and_monitoring(&self) -> Result<(), GraphError> {
        if self.monitoring.direct
            && self.monitoring.software
            && !self.monitoring.doubled_path_confirmed
        {
            return Err(GraphError::new(
                "direct and software monitoring require confirmation",
            ));
        }
        let mut ports = BTreeSet::new();
        for pair in self.nodes.iter().filter_map(|node| match &node.kind {
            NodeKind::Source {
                source: SourceKind::LiveInput { ports },
            }
            | NodeKind::Source {
                source: SourceKind::HardwareReturn { ports, .. },
            }
            | NodeKind::Sink {
                sink: SinkKind::MainPlayback { ports },
            }
            | NodeKind::Sink {
                sink: SinkKind::HardwareSend { ports, .. },
            } => Some(ports),
            _ => None,
        }) {
            if pair.left.trim().is_empty()
                || pair.right.trim().is_empty()
                || pair.left == pair.right
            {
                return Err(GraphError::new(
                    "stereo JACK ports must be exact and distinct",
                ));
            }
            if !ports.insert(pair.left.as_str()) || !ports.insert(pair.right.as_str()) {
                return Err(GraphError::new("ambiguous physical JACK port assignment"));
            }
        }
        Ok(())
    }

    fn topological_order(
        &self,
        nodes: &BTreeMap<NodeId, &Node>,
    ) -> Result<Vec<NodeId>, GraphError> {
        let mut indegree = nodes
            .keys()
            .map(|id| (*id, 0usize))
            .collect::<BTreeMap<_, _>>();
        let mut outgoing = BTreeMap::<NodeId, Vec<NodeId>>::new();
        let mut edge_ids = BTreeSet::new();
        let mut edge_routes = BTreeSet::new();
        let mut outdegree = BTreeMap::<NodeId, usize>::new();
        for edge in &self.edges {
            if edge.id == 0 || !edge_ids.insert(edge.id) {
                return Err(GraphError::new("edge IDs must be unique and non-zero"));
            }
            if edge.from == edge.to {
                return Err(GraphError::new("self-edge is not allowed"));
            }
            if !edge_routes.insert((edge.from, edge.to)) {
                return Err(GraphError::new("duplicate audio edge is not allowed"));
            }
            let from = nodes
                .get(&edge.from)
                .ok_or_else(|| GraphError::new("edge source is missing"))?;
            let to = nodes
                .get(&edge.to)
                .ok_or_else(|| GraphError::new("edge destination is missing"))?;
            let from_layout = from.layout;
            let accepts_mono = matches!(to.kind, NodeKind::MonoToStereo);
            if from_layout != to.layout && !(from_layout == ChannelLayout::Mono && accepts_mono) {
                return Err(GraphError::new("edge channel layout mismatch"));
            }
            *indegree.get_mut(&edge.to).expect("validated node") += 1;
            *outdegree.entry(edge.from).or_default() += 1;
            outgoing.entry(edge.from).or_default().push(edge.to);
        }
        for node in nodes.values() {
            if matches!(node.kind, NodeKind::AuxReturn { .. })
                && outdegree.get(&node.id).copied() != Some(1)
            {
                return Err(GraphError::new("an aux return must be mixed exactly once"));
            }
        }
        for targets in outgoing.values_mut() {
            targets.sort_unstable();
        }
        let mut ready = indegree
            .iter()
            .filter_map(|(id, count)| (*count == 0).then_some(*id))
            .collect::<BTreeSet<_>>();
        let mut order = Vec::with_capacity(nodes.len());
        while let Some(id) = ready.pop_first() {
            order.push(id);
            if let Some(targets) = outgoing.get(&id) {
                for target in targets {
                    let count = indegree.get_mut(target).expect("validated node");
                    *count -= 1;
                    if *count == 0 {
                        ready.insert(*target);
                    }
                }
            }
        }
        if order.len() != nodes.len() {
            return Err(GraphError::new("audio graph contains a cycle"));
        }
        Ok(order)
    }

    fn validate_hardware_cycles(
        &self,
        nodes: &BTreeMap<NodeId, &Node>,
        _order: &[NodeId],
    ) -> Result<(), GraphError> {
        let adjacency =
            self.edges
                .iter()
                .fold(BTreeMap::<NodeId, Vec<NodeId>>::new(), |mut map, edge| {
                    map.entry(edge.from).or_default().push(edge.to);
                    map
                });
        for (return_id, loop_id) in nodes.iter().filter_map(|(id, node)| match &node.kind {
            NodeKind::Source {
                source: SourceKind::HardwareReturn { loop_id, .. },
            } => Some((*id, loop_id)),
            _ => None,
        }) {
            let sends = nodes
                .iter()
                .filter_map(|(id, node)| match &node.kind {
                    NodeKind::Sink {
                        sink:
                            SinkKind::HardwareSend {
                                loop_id: send_loop, ..
                            },
                    } if send_loop == loop_id => Some(*id),
                    _ => None,
                })
                .collect::<BTreeSet<_>>();
            if reaches_any(return_id, &sends, &adjacency) {
                return Err(GraphError::new("hardware return feeds its own send"));
            }
        }
        Ok(())
    }
}

fn validate_chain(
    chain: &[EffectId],
    effects: &BTreeMap<EffectId, &EffectInstance>,
    used: &mut BTreeSet<EffectId>,
) -> Result<(), GraphError> {
    if chain.len() > MAX_CHAIN_EFFECTS {
        return Err(GraphError::new("serial effect chain bound exceeded"));
    }
    for effect in chain {
        if !effects.contains_key(effect) {
            return Err(GraphError::new("chain refers to a missing effect"));
        }
        if !used.insert(*effect) {
            return Err(GraphError::new(
                "effect instance appears in more than one chain",
            ));
        }
    }
    Ok(())
}

fn reaches_any(
    start: NodeId,
    targets: &BTreeSet<NodeId>,
    adjacency: &BTreeMap<NodeId, Vec<NodeId>>,
) -> bool {
    let mut stack = vec![start];
    let mut visited = BTreeSet::new();
    while let Some(node) = stack.pop() {
        if !visited.insert(node) {
            continue;
        }
        if targets.contains(&node) {
            return true;
        }
        if let Some(next) = adjacency.get(&node) {
            stack.extend(next.iter().copied());
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dry_graph() -> GraphDefinition {
        GraphDefinition {
            format_version: GRAPH_FORMAT_VERSION,
            enabled: true,
            sample_rate: 48_000,
            maximum_callback_frames: 128,
            nodes: vec![
                Node {
                    id: 1,
                    layout: ChannelLayout::Stereo,
                    kind: NodeKind::Source {
                        source: SourceKind::ManagedEngine,
                    },
                },
                Node {
                    id: 2,
                    layout: ChannelLayout::Stereo,
                    kind: NodeKind::StereoMixer,
                },
                Node {
                    id: 3,
                    layout: ChannelLayout::Stereo,
                    kind: NodeKind::Sink {
                        sink: SinkKind::MainPlayback {
                            ports: StereoPorts {
                                left: "system:playback_1".into(),
                                right: "system:playback_2".into(),
                            },
                        },
                    },
                },
            ],
            edges: vec![
                Edge {
                    id: 1,
                    from: 1,
                    to: 2,
                },
                Edge {
                    id: 2,
                    from: 2,
                    to: 3,
                },
            ],
            effects: vec![],
            source_chains: vec![SourceChain {
                source_node: 1,
                effects: vec![],
            }],
            master_chain: vec![],
            aux_buses: vec![],
            sends: vec![],
            monitoring: Monitoring::default(),
            recording_tap: RecordingTap::PostMaster,
        }
    }

    #[test]
    fn dry_graph_has_deterministic_topology_and_round_trips() {
        let graph = dry_graph();
        assert_eq!(graph.validate().unwrap(), [1, 2, 3]);
        let encoded = serde_json::to_vec(&graph).unwrap();
        let decoded: GraphDefinition = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, graph);
    }

    #[test]
    fn insert_rack_allocates_stable_ids_and_reorders_without_recreating_effects() {
        let mut rack = InsertRack::default();
        let eq = rack.add(EffectKind::Eq).unwrap();
        let compressor = rack.add(EffectKind::Compressor).unwrap();
        let crusher = rack.add(EffectKind::Crusher).unwrap();
        rack.effect_mut(compressor)
            .unwrap()
            .parameters
            .insert("threshold_db".into(), -24.0);

        rack.move_to(compressor, 0).unwrap();
        assert_eq!(rack.order, [compressor, eq, crusher]);
        assert_eq!(
            rack.effect(compressor).unwrap().parameters["threshold_db"],
            -24.0
        );
        let removed = rack.remove(eq).unwrap();
        assert_eq!(removed.id, eq);
        assert_eq!(rack.add(EffectKind::Gate).unwrap(), crusher + 1);
        rack.validate().unwrap();
    }

    #[test]
    fn insert_rack_rejects_unknown_duplicate_and_chain_overflow() {
        let mut rack = InsertRack::default();
        let id = rack.add(EffectKind::Eq).unwrap();
        rack.order.push(id);
        assert!(rack.validate().is_err());

        let mut rack = InsertRack::default();
        assert!(rack.add(EffectKind::Reverb).is_err());
        for _ in 0..MAX_CHAIN_EFFECTS {
            rack.add(EffectKind::Utility).unwrap();
        }
        assert!(rack.add(EffectKind::Utility).is_err());
    }

    #[test]
    fn cycles_missing_nodes_duplicates_and_layout_mismatch_are_rejected() {
        let mut graph = dry_graph();
        graph.edges.push(Edge {
            id: 3,
            from: 3,
            to: 1,
        });
        assert!(graph.validate().unwrap_err().to_string().contains("cycle"));
        let mut graph = dry_graph();
        graph.edges[0].to = 99;
        assert!(graph.validate().is_err());
        let mut graph = dry_graph();
        graph.nodes[1].id = 1;
        assert!(graph.validate().is_err());
        let mut graph = dry_graph();
        graph.nodes[0].layout = ChannelLayout::Mono;
        assert!(graph.validate().is_err());
    }

    #[test]
    fn all_hard_bounds_and_non_finite_parameters_are_rejected() {
        let mut graph = dry_graph();
        graph.maximum_callback_frames = MAX_CALLBACK_FRAMES + 1;
        assert!(graph.validate().is_err());
        let mut graph = dry_graph();
        graph
            .nodes
            .extend((4..=MAX_NODES as u32 + 1).map(|id| Node {
                id,
                layout: ChannelLayout::Stereo,
                kind: NodeKind::StereoMixer,
            }));
        assert!(graph.validate().is_err());
        let mut graph = dry_graph();
        graph.effects.push(EffectInstance {
            id: 1,
            kind: EffectKind::Utility,
            version: 1,
            bypass: false,
            parameters: BTreeMap::from([("trim_db".into(), f32::NAN)]),
            owned_memory_bytes: 0,
        });
        assert!(graph.validate().is_err());
        let mut graph = dry_graph();
        graph.effects.push(EffectInstance {
            id: 1,
            kind: EffectKind::Delay,
            version: 1,
            bypass: false,
            parameters: BTreeMap::new(),
            owned_memory_bytes: MAX_EFFECT_MEMORY_BYTES + 1,
        });
        assert!(graph.validate().is_err());
    }

    #[test]
    fn unsafe_monitor_ports_hardware_loop_and_dry_aux_are_rejected() {
        let mut graph = dry_graph();
        graph.monitoring = Monitoring {
            direct: true,
            software: true,
            doubled_path_confirmed: false,
        };
        assert!(graph.validate().is_err());
        let mut graph = dry_graph();
        if let NodeKind::Sink {
            sink: SinkKind::MainPlayback { ports },
        } = &mut graph.nodes[2].kind
        {
            ports.right = ports.left.clone();
        }
        assert!(graph.validate().is_err());

        let ports = StereoPorts {
            left: "capture:1".into(),
            right: "capture:2".into(),
        };
        let mut graph = dry_graph();
        graph.nodes[0].kind = NodeKind::Source {
            source: SourceKind::HardwareReturn {
                loop_id: "pedal".into(),
                ports,
            },
        };
        graph.nodes[2].kind = NodeKind::Sink {
            sink: SinkKind::HardwareSend {
                loop_id: "pedal".into(),
                ports: StereoPorts {
                    left: "send:1".into(),
                    right: "send:2".into(),
                },
            },
        };
        assert!(graph
            .validate()
            .unwrap_err()
            .to_string()
            .contains("own send"));

        let mut graph = dry_graph();
        graph.effects.push(EffectInstance {
            id: 1,
            kind: EffectKind::Delay,
            version: 1,
            bypass: false,
            parameters: BTreeMap::from([("dry_percent".into(), 50.0)]),
            owned_memory_bytes: 1024,
        });
        graph.aux_buses.push(AuxBus {
            id: 1,
            effects: vec![1],
            return_gain_db: 0.0,
        });
        assert!(graph
            .validate()
            .unwrap_err()
            .to_string()
            .contains("100% wet"));
    }

    #[test]
    fn future_graph_and_effect_versions_are_refused() {
        let mut graph = dry_graph();
        graph.format_version += 1;
        assert!(graph.validate().is_err());
        let mut graph = dry_graph();
        graph.effects.push(EffectInstance {
            id: 1,
            kind: EffectKind::Utility,
            version: 2,
            bypass: false,
            parameters: BTreeMap::new(),
            owned_memory_bytes: 0,
        });
        assert!(graph.validate().is_err());
    }
}
