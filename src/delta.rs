//! Incremental diff model.
//!
//! A [`Delta`] describes **only** what changed between two consecutive
//! timeline nodes.  Replaying all deltas from the root node reconstructs
//! the full entity-pool state at any point in time without storing redundant
//! copies of unchanged data.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::entity::{Properties, Value};
use crate::types::EntityId;

// Re-exported so callers can build deltas without reaching into sub-modules.
pub use crate::entity::Entity;

/// Changes to an entity's key-value properties.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PropertyDiff {
    /// Properties that were added or whose value changed.
    pub upserted: Properties,
    /// Property keys that were removed.
    pub removed: Vec<String>,
}

/// Changes to an entity's direct relationships.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RelationshipDiff {
    pub added: Vec<EntityId>,
    pub removed: Vec<EntityId>,
}

/// Changes to the roles an entity fills.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RoleDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

/// All patches applied to a single existing entity in one time step.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EntityDiff {
    pub property_diff: PropertyDiff,
    pub relationship_diff: RelationshipDiff,
    pub role_diff: RoleDiff,
    /// `Some(false)` = entity died; `Some(true)` = entity revived.
    pub alive_change: Option<bool>,
}

impl EntityDiff {
    /// Convenience constructor with empty/default fields.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Merge `other` into `self`, with `other` taking precedence for
    /// conflicting property / alive changes.
    pub fn merge_from(&mut self, other: &EntityDiff) {
        for (k, v) in &other.property_diff.upserted {
            self.property_diff.upserted.insert(k.clone(), v.clone());
        }
        self.property_diff
            .removed
            .extend(other.property_diff.removed.iter().cloned());

        self.relationship_diff
            .added
            .extend(other.relationship_diff.added.iter().cloned());
        self.relationship_diff
            .removed
            .extend(other.relationship_diff.removed.iter().cloned());

        self.role_diff
            .added
            .extend(other.role_diff.added.iter().cloned());
        self.role_diff
            .removed
            .extend(other.role_diff.removed.iter().cloned());

        if let Some(alive) = other.alive_change {
            self.alive_change = Some(alive);
        }
    }
}

/// The complete set of world changes occurring in one timeline step.
///
/// Storing only the delta (not a full snapshot) keeps memory use proportional
/// to the *volume of change* rather than the total world size.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Delta {
    /// Brand-new entities introduced this step.
    pub added_entities: HashMap<EntityId, Entity>,
    /// Patches to entities that already existed.
    pub modified_entities: HashMap<EntityId, EntityDiff>,
    /// Entities permanently removed from the pool.
    pub removed_entities: HashSet<EntityId>,
    /// Causal events **triggered** (not yet applied) this step.
    ///
    /// Events with `propagation_delay == 0` are applied immediately;
    /// those with a positive delay are queued by the manager and baked
    /// into a future delta when they mature.
    pub events: Vec<crate::causal::CausalEvent>,
}

impl Delta {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.added_entities.is_empty()
            && self.modified_entities.is_empty()
            && self.removed_entities.is_empty()
            && self.events.is_empty()
    }

    /// Absorb all changes from `other` into `self`.
    ///
    /// Conflicts are resolved in favour of `other` (later change wins).
    pub fn merge_from(&mut self, other: &Delta) {
        for (id, entity) in &other.added_entities {
            self.added_entities.insert(*id, entity.clone());
        }
        for (id, diff) in &other.modified_entities {
            self.modified_entities
                .entry(*id)
                .or_default()
                .merge_from(diff);
        }
        self.removed_entities
            .extend(other.removed_entities.iter().copied());
        self.events.extend(other.events.iter().cloned());
    }

    /// Return the entry for `id` in `modified_entities`, creating an
    /// empty one if absent.
    pub fn entity_diff_mut(&mut self, id: EntityId) -> &mut EntityDiff {
        self.modified_entities.entry(id).or_default()
    }

    // ---- convenience builder methods --------------------------------

    /// Stage an entity to be added.
    pub fn add_entity(mut self, entity: Entity) -> Self {
        self.added_entities.insert(entity.id, entity);
        self
    }

    /// Stage an entity death.
    pub fn kill_entity(mut self, id: EntityId) -> Self {
        self.entity_diff_mut(id).alive_change = Some(false);
        self
    }

    /// Stage a property upsert on an existing entity.
    pub fn set_property(mut self, id: EntityId, key: impl Into<String>, value: Value) -> Self {
        self.entity_diff_mut(id)
            .property_diff
            .upserted
            .insert(key.into(), value);
        self
    }

    /// Stage a role assignment on an existing entity.
    pub fn assign_role(mut self, id: EntityId, role: impl Into<String>) -> Self {
        self.entity_diff_mut(id).role_diff.added.push(role.into());
        self
    }

    /// Stage a relationship addition between two entities.
    pub fn add_relationship(mut self, from: EntityId, to: EntityId) -> Self {
        self.entity_diff_mut(from).relationship_diff.added.push(to);
        self
    }

    /// Attach a causal event that will be queued for propagation.
    pub fn with_event(mut self, event: crate::causal::CausalEvent) -> Self {
        self.events.push(event);
        self
    }
}
