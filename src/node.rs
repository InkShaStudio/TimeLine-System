//! Timeline node: a single immutable snapshot-step in the timeline.
//!
//! Each node stores only the *incremental delta* from its parent, not a
//! full copy of the world state.  To reconstruct world state at any node,
//! replay deltas from the root (or from the nearest cached checkpoint).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::delta::Delta;
use crate::types::{NodeId, TimelineId};

/// One immutable step in a timeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineNode {
    pub id: NodeId,
    pub timeline_id: TimelineId,
    /// `None` only for the root node.
    pub parent_id: Option<NodeId>,
    /// Monotonically increasing logical clock within the timeline branch.
    pub timestamp: u64,
    /// All world changes that occurred at this step (user input + causal
    /// effects + tendency corrections, all merged together).
    pub delta: Delta,
    /// Human-readable summary of what happened.
    pub description: String,
    /// Arbitrary key/value annotations (e.g. author, tags).
    pub metadata: HashMap<String, String>,
}

impl TimelineNode {
    /// Create the root node for a new timeline (timestamp 0, empty delta).
    pub fn root(timeline_id: TimelineId) -> Self {
        Self {
            id: Uuid::new_v4(),
            timeline_id,
            parent_id: None,
            timestamp: 0,
            delta: Delta::new(),
            description: "Root".to_string(),
            metadata: HashMap::new(),
        }
    }

    /// Create a non-root node.
    pub fn new(
        timeline_id: TimelineId,
        parent_id: NodeId,
        timestamp: u64,
        delta: Delta,
        description: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            timeline_id,
            parent_id: Some(parent_id),
            timestamp,
            delta,
            description: description.into(),
            metadata: HashMap::new(),
        }
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}
