//! Timeline: a labelled, linearly-ordered sequence of node IDs.
//!
//! The "main" timeline is created automatically by [`crate::manager::TimelineManager::new`].
//! Branches are created by [`crate::manager::TimelineManager::branch_at`].
//! Time-travel creates a special kind of branch via
//! [`crate::manager::TimelineManager::time_travel_to`] that is immune to the
//! grandfather paradox because it never modifies the original timeline.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::{NodeId, TimelineId};

/// A single timeline: the main trunk or any branch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Timeline {
    pub id: TimelineId,
    /// The timeline this was forked from (`None` = main/root timeline).
    pub parent_id: Option<TimelineId>,
    /// The node in the parent timeline at which the fork occurred.
    pub branch_point: Option<NodeId>,
    /// Ordered list of node IDs **owned** by this timeline.
    ///
    /// Nodes inherited from the parent timeline (before the branch point)
    /// are **not** stored here; use [`crate::manager::TimelineManager::full_history`]
    /// to get the complete ancestor chain.
    pub nodes: Vec<NodeId>,
    /// Display name.
    pub name: String,
    /// `true` when created by a time-travel operation.
    pub is_time_travel_branch: bool,
}

impl Timeline {
    /// Create the root (main) timeline.
    pub fn new_root() -> Self {
        Self {
            id: Uuid::new_v4(),
            parent_id: None,
            branch_point: None,
            nodes: Vec::new(),
            name: "Main Timeline".to_string(),
            is_time_travel_branch: false,
        }
    }

    /// Fork from an existing timeline at a specific node.
    pub fn branch_from(
        parent_id: TimelineId,
        branch_point: NodeId,
        name: impl Into<String>,
        is_time_travel_branch: bool,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            parent_id: Some(parent_id),
            branch_point: Some(branch_point),
            nodes: Vec::new(),
            name: name.into(),
            is_time_travel_branch,
        }
    }

    /// The most recent node ID in this timeline, if any.
    pub fn current_node(&self) -> Option<NodeId> {
        self.nodes.last().copied()
    }

    /// Number of nodes owned by this timeline (not counting inherited ancestors).
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}
