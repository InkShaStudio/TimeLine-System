//! [`TimelineManager`]: the central coordinator of the timeline system.
//!
//! Responsibilities:
//! * Advancing time (applying user deltas, maturing causal events, running
//!   world tendencies) and recording each step as an immutable [`TimelineNode`].
//! * Branching and time-travel (creating independent child timelines).
//! * Efficient state reconstruction at any past node via delta replay and
//!   periodic state caching.
//! * Maintaining the [`EntityPool`] in sync with the active timeline.

use std::collections::HashMap;

use crate::causal::{apply_effect_into_delta, CausalEvent};
use crate::delta::Delta;
use crate::entity::EntityPool;
use crate::node::TimelineNode;
use crate::tendency::Tendency;
use crate::timeline::Timeline;
use crate::types::{NodeId, TimelineId};

/// The timeline system's top-level coordinator.
///
/// # Functional / pure-snapshot model
///
/// Each call to [`advance`](Self::advance) creates one new immutable
/// [`TimelineNode`].  The node's delta is the merge of:
/// 1. Any matured causal-event effects.
/// 2. The caller-supplied input delta.
/// 3. The corrective deltas produced by world tendencies.
///
/// Because everything is merged into a single stored delta, replaying nodes
/// in order from root faithfully reconstructs any past state.
///
/// # Time travel (grandfather-paradox-free)
///
/// [`time_travel_to`](Self::time_travel_to) does **not** alter the existing
/// timeline.  Instead it creates a fresh child timeline whose ancestry starts
/// at the specified past node.  The original timeline is frozen in place.
pub struct TimelineManager {
    entity_pool: EntityPool,
    timelines: HashMap<TimelineId, Timeline>,
    nodes: HashMap<NodeId, TimelineNode>,
    active_timeline_id: TimelineId,
    tendencies: Vec<Tendency>,
    /// Pending causal events: `(event, remaining_delay_steps)`.
    pending_events: Vec<(CausalEvent, u64)>,
    /// Periodic full-state snapshots for O(log n) reconstruction.
    state_cache: HashMap<NodeId, EntityPool>,
    /// Cache a full snapshot every N nodes.
    cache_interval: usize,
    /// Counter incremented on each `advance` call.
    node_count: usize,
}

impl TimelineManager {
    /// Create a new manager with an empty world and a single root timeline.
    pub fn new() -> Self {
        let main_timeline = Timeline::new_root();
        let timeline_id = main_timeline.id;

        let root_node = TimelineNode::root(timeline_id);
        let root_node_id = root_node.id;

        let mut timelines = HashMap::new();
        timelines.insert(timeline_id, main_timeline);
        timelines
            .get_mut(&timeline_id)
            .unwrap()
            .nodes
            .push(root_node_id);

        let mut nodes = HashMap::new();
        nodes.insert(root_node_id, root_node);

        let empty_pool = EntityPool::new();
        let mut state_cache = HashMap::new();
        state_cache.insert(root_node_id, empty_pool.clone());

        Self {
            entity_pool: empty_pool,
            timelines,
            nodes,
            active_timeline_id: timeline_id,
            tendencies: Vec::new(),
            pending_events: Vec::new(),
            state_cache,
            cache_interval: 10,
            node_count: 0,
        }
    }

    // ------------------------------------------------------------------ //
    // Accessors                                                            //
    // ------------------------------------------------------------------ //

    pub fn active_timeline(&self) -> &Timeline {
        self.timelines.get(&self.active_timeline_id).unwrap()
    }

    pub fn active_timeline_id(&self) -> TimelineId {
        self.active_timeline_id
    }

    pub fn entity_pool(&self) -> &EntityPool {
        &self.entity_pool
    }

    pub fn get_timeline(&self, id: TimelineId) -> Option<&Timeline> {
        self.timelines.get(&id)
    }

    pub fn get_node(&self, id: NodeId) -> Option<&TimelineNode> {
        self.nodes.get(&id)
    }

    pub fn current_node(&self) -> Option<&TimelineNode> {
        let id = self.active_timeline().current_node()?;
        self.nodes.get(&id)
    }

    pub fn timelines(&self) -> impl Iterator<Item = &Timeline> {
        self.timelines.values()
    }

    // ------------------------------------------------------------------ //
    // Tendency management                                                  //
    // ------------------------------------------------------------------ //

    pub fn add_tendency(&mut self, tendency: Tendency) {
        self.tendencies.push(tendency);
    }

    pub fn remove_tendency(&mut self, name: &str) {
        self.tendencies.retain(|t| t.name != name);
    }

    pub fn tendencies(&self) -> &[Tendency] {
        &self.tendencies
    }

    // ------------------------------------------------------------------ //
    // Time advancement                                                     //
    // ------------------------------------------------------------------ //

    /// Advance the active timeline by one step.
    ///
    /// The `input_delta` represents the *intentional* world changes for this
    /// step.  The manager augments it with:
    /// * effects of any causal events whose delay counter reached zero,
    /// * effects of `delay=0` events from the input (applied in the same step),
    /// * corrective deltas from world tendencies (evaluated after the input
    ///   is applied, in descending priority order).
    ///
    /// ## Delay semantics
    /// * `propagation_delay = 0` → effects applied **in the same `advance` call**.
    /// * `propagation_delay = N` → delay counter decremented once per `advance`;
    ///   effects applied in the call where the counter **reaches 0** (i.e. after
    ///   exactly N subsequent calls).
    ///
    /// All changes are merged into one [`Delta`] that is stored in the new
    /// [`TimelineNode`].  Returns the new node's ID.
    pub fn advance(&mut self, input_delta: Delta, description: impl Into<String>) -> NodeId {
        let mut merged = Delta::new();

        // 1. Decrement pending event delays; collect those that just matured.
        for (_, d) in &mut self.pending_events {
            if *d > 0 {
                *d -= 1;
            }
        }
        let matured: Vec<CausalEvent> = self
            .pending_events
            .iter()
            .filter(|(_, d)| *d == 0)
            .map(|(e, _)| e.clone())
            .collect();
        self.pending_events.retain(|(_, d)| *d > 0);

        // Apply matured effects into the merged delta.
        for event in &matured {
            for effect in &event.effects {
                apply_effect_into_delta(&mut merged, effect);
            }
        }

        // 2. Merge caller's entity/relationship/role changes (non-event parts).
        merged.merge_from(&input_delta);

        // 3. Process caller's causal events.
        //    delay=0 → apply immediately in this step.
        //    delay>0 → queue for decrement in future steps.
        for event in &input_delta.events {
            if event.propagation_delay == 0 {
                for effect in &event.effects {
                    apply_effect_into_delta(&mut merged, effect);
                }
            } else {
                self.pending_events
                    .push((event.clone(), event.propagation_delay));
            }
        }

        // 4. Apply all accumulated changes to the live pool.
        self.entity_pool.apply_delta(&merged);

        // 5. Evaluate world tendencies (in priority order, each step sees
        //    the pool as modified by all previous tendency corrections).
        let tendencies = self.tendencies.clone(); // clone to satisfy borrow checker
        let mut sorted = tendencies;
        sorted.sort_by(|a, b| b.priority.cmp(&a.priority));

        for tendency in &sorted {
            if let Some(td) = tendency.generate_delta(&self.entity_pool) {
                self.entity_pool.apply_delta(&td);
                merged.merge_from(&td);
            }
        }

        // 6. Commit the node.
        let timeline_id = self.active_timeline_id;
        let parent_id = self
            .active_timeline()
            .current_node()
            .expect("timeline must have at least a root node");
        let timestamp = self
            .nodes
            .get(&parent_id)
            .map(|n| n.timestamp + 1)
            .unwrap_or(1);

        let node = TimelineNode::new(timeline_id, parent_id, timestamp, merged, description);
        let node_id = node.id;
        self.nodes.insert(node_id, node);
        self.timelines
            .get_mut(&timeline_id)
            .unwrap()
            .nodes
            .push(node_id);

        // Periodically cache a full copy for fast reconstruction.
        self.node_count += 1;
        if self.node_count.is_multiple_of(self.cache_interval) {
            self.state_cache
                .insert(node_id, self.entity_pool.clone());
        }

        node_id
    }

    // ------------------------------------------------------------------ //
    // Branching                                                            //
    // ------------------------------------------------------------------ //

    /// Create a standard branch timeline starting from `node_id`.
    ///
    /// The new timeline is set as the active timeline.  Returns the new
    /// timeline's ID or an error if `node_id` is unknown.
    pub fn branch_at(
        &mut self,
        node_id: NodeId,
        name: impl Into<String>,
    ) -> Result<TimelineId, String> {
        if !self.nodes.contains_key(&node_id) {
            return Err(format!("Node {node_id} not found"));
        }
        let parent_id = self.active_timeline_id;
        let new_timeline = Timeline::branch_from(parent_id, node_id, name, false);
        let new_id = new_timeline.id;
        self.timelines.insert(new_id, new_timeline);

        // Add the branch-point node to the new timeline so it has a "current" node.
        self.timelines.get_mut(&new_id).unwrap().nodes.push(node_id);

        self.active_timeline_id = new_id;
        self.entity_pool = self.reconstruct_state_at(node_id);
        Ok(new_id)
    }

    /// Create a **time-travel** branch rooted at a past node.
    ///
    /// Unlike a normal branch this is marked `is_time_travel_branch = true`
    /// and does *not* alter the source timeline (no grandfather paradox).
    pub fn time_travel_to(&mut self, node_id: NodeId) -> Result<TimelineId, String> {
        if !self.nodes.contains_key(&node_id) {
            return Err(format!("Node {node_id} not found"));
        }
        let parent_id = self.active_timeline_id;
        let new_timeline =
            Timeline::branch_from(parent_id, node_id, "Time Travel Branch", true);
        let new_id = new_timeline.id;
        self.timelines.insert(new_id, new_timeline);

        // Start the new branch's node list at the travel target.
        self.timelines.get_mut(&new_id).unwrap().nodes.push(node_id);

        self.active_timeline_id = new_id;
        self.entity_pool = self.reconstruct_state_at(node_id);

        // Pending causal events from the source timeline are intentionally
        // NOT carried over; the new branch starts clean from the world state
        // at the target node.
        self.pending_events.clear();

        Ok(new_id)
    }

    /// Switch the active timeline to an existing one (by ID).
    pub fn switch_timeline(&mut self, timeline_id: TimelineId) -> Result<(), String> {
        if !self.timelines.contains_key(&timeline_id) {
            return Err(format!("Timeline {timeline_id} not found"));
        }
        self.active_timeline_id = timeline_id;
        if let Some(current) = self.active_timeline().current_node() {
            self.entity_pool = self.reconstruct_state_at(current);
        } else {
            self.entity_pool = EntityPool::new();
        }
        Ok(())
    }

    // ------------------------------------------------------------------ //
    // State reconstruction                                                 //
    // ------------------------------------------------------------------ //

    /// Reconstruct the full [`EntityPool`] state that existed at `node_id`.
    ///
    /// Uses the nearest cached snapshot as a starting point to minimise the
    /// number of deltas that must be replayed.
    pub fn reconstruct_state_at(&self, node_id: NodeId) -> EntityPool {
        if let Some(cached) = self.state_cache.get(&node_id) {
            return cached.clone();
        }

        let chain = self.ancestor_chain(node_id);

        // Find the newest cached ancestor.
        let mut base = EntityPool::new();
        let mut start = 0usize;
        for (i, &nid) in chain.iter().enumerate() {
            if let Some(cached) = self.state_cache.get(&nid) {
                base = cached.clone();
                start = i + 1;
            }
        }

        // Replay remaining deltas.
        for &nid in &chain[start..] {
            if let Some(node) = self.nodes.get(&nid) {
                base.apply_delta(&node.delta);
            }
        }
        base
    }

    /// Ordered chain of node IDs from the root to `target` (inclusive).
    fn ancestor_chain(&self, target: NodeId) -> Vec<NodeId> {
        let mut chain = vec![target];
        let mut current = target;
        while let Some(node) = self.nodes.get(&current) {
            match node.parent_id {
                Some(pid) => {
                    chain.push(pid);
                    current = pid;
                }
                None => break,
            }
        }
        chain.reverse();
        chain
    }

    /// All nodes in the active timeline's own `nodes` list, in order.
    pub fn active_timeline_nodes(&self) -> Vec<&TimelineNode> {
        self.active_timeline()
            .nodes
            .iter()
            .filter_map(|id| self.nodes.get(id))
            .collect()
    }

    /// Full ancestor chain of nodes for the active timeline, from root to tip.
    pub fn full_history(&self) -> Vec<&TimelineNode> {
        if let Some(tip) = self.active_timeline().current_node() {
            self.ancestor_chain(tip)
                .into_iter()
                .filter_map(|id| self.nodes.get(&id))
                .collect()
        } else {
            vec![]
        }
    }

    // ------------------------------------------------------------------ //
    // Diagnostics                                                          //
    // ------------------------------------------------------------------ //

    /// Number of pending causal events waiting to mature.
    pub fn pending_event_count(&self) -> usize {
        self.pending_events.len()
    }

    /// Override the cache interval (default: every 10 nodes).
    pub fn set_cache_interval(&mut self, interval: usize) {
        self.cache_interval = interval.max(1);
    }
}

impl Default for TimelineManager {
    fn default() -> Self {
        Self::new()
    }
}
