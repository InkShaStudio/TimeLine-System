//! World-tendency model: persistent forces that self-correct the timeline.
//!
//! A [`Tendency`] expresses an invariant of the world (e.g. *"there must
//! always be a unifier"*).  The [`crate::manager::TimelineManager`] evaluates
//! all tendencies after each `advance` call and, if their condition is met,
//! generates a corrective [`crate::delta::Delta`] automatically.
//!
//! ## Self-correction example
//! If Zhang San held the `"unifier"` role and is killed, the
//! `RoleVacant { role: "unifier" }` condition fires and the
//! `FillRole { role: "unifier", ... }` effect promotes the next available
//! hero (Li Si) into that role – preserving the tendency toward unification.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::entity::{Entity, EntityPool, Value};
use crate::types::TendencyId;

/// Condition that, when true, triggers a tendency's corrective effect.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TendencyCondition {
    /// The role has no alive occupants.
    RoleVacant { role: String },
    /// Fewer than `threshold` alive entities of the given kind exist.
    EntityCountBelow { kind: String, threshold: usize },
    /// More than `threshold` alive entities of the given kind exist.
    EntityCountAbove { kind: String, threshold: usize },
    /// Fewer than `threshold` alive entities fill the given role.
    RoleCountBelow { role: String, threshold: usize },
    /// At least one alive entity of `kind` has `property == value`.
    PropertyEquals {
        kind: String,
        property: String,
        value: Value,
    },
    /// Always fires (useful for logging / unconditional world rules).
    Always,
}

impl TendencyCondition {
    /// Evaluate the condition against the current entity pool.
    pub fn is_met(&self, pool: &EntityPool) -> bool {
        match self {
            TendencyCondition::RoleVacant { role } => pool.is_role_vacant(role),
            TendencyCondition::EntityCountBelow { kind, threshold } => {
                pool.get_by_kind(kind).len() < *threshold
            }
            TendencyCondition::EntityCountAbove { kind, threshold } => {
                pool.get_by_kind(kind).len() > *threshold
            }
            TendencyCondition::RoleCountBelow { role, threshold } => {
                pool.get_by_role(role).len() < *threshold
            }
            TendencyCondition::PropertyEquals {
                kind,
                property,
                value,
            } => pool
                .get_by_kind(kind)
                .iter()
                .any(|e| e.properties.get(property) == Some(value)),
            TendencyCondition::Always => true,
        }
    }
}

/// The corrective action taken when a tendency's condition is true.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TendencyEffect {
    /// Promote an existing entity to fill a vacant role.
    ///
    /// Selects the first available alive entity of `source_kind` (or any
    /// kind if `None`) that does not already have the role.  Optionally
    /// sets a property on the promoted entity.
    FillRole {
        role: String,
        source_kind: Option<String>,
        /// `(property_name, value)` set on the promoted entity.
        promoted_property: Option<(String, Value)>,
    },
    /// Unconditionally spawn a new entity with the specified attributes.
    SpawnEntity {
        kind: String,
        roles: Vec<String>,
        initial_properties: HashMap<String, Value>,
    },
    /// Ensure at least `min_count` alive entities of `kind` exist;
    /// spawn as many as needed, assigning the listed roles.
    EnsureEntityCount {
        kind: String,
        min_count: usize,
        roles: Vec<String>,
    },
    /// Remove the role from any entities beyond the first `keep_count`.
    TrimRole { role: String, keep_count: usize },
}

/// A world-level tendency that provides self-correcting behaviour.
///
/// Tendencies are evaluated (in descending priority order) after every call
/// to [`crate::manager::TimelineManager::advance`].  Their effects are merged
/// into the node's delta so that state reconstruction by replay is accurate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tendency {
    pub id: TendencyId,
    /// Human-readable identifier used to reference / remove the tendency.
    pub name: String,
    pub description: String,
    pub condition: TendencyCondition,
    pub effect: TendencyEffect,
    /// Probability in `[0.0, 1.0]` that the tendency fires when its
    /// condition is met.  Defaults to `1.0` (always fires).
    pub strength: f64,
    /// Evaluation order: higher = evaluated first.
    pub priority: i32,
}

impl Tendency {
    /// Build a tendency that always fires (`strength = 1.0`, `priority = 0`).
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        condition: TendencyCondition,
        effect: TendencyEffect,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            description: description.into(),
            condition,
            effect,
            strength: 1.0,
            priority: 0,
        }
    }

    pub fn with_strength(mut self, strength: f64) -> Self {
        self.strength = strength.clamp(0.0, 1.0);
        self
    }

    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    /// Whether the tendency's condition is currently satisfied.
    pub fn condition_met(&self, pool: &EntityPool) -> bool {
        self.condition.is_met(pool)
    }

    /// Generate the corrective delta for this tendency, given the current
    /// pool state.  Returns `None` if the condition is not met or no
    /// candidate entity can be found.
    pub fn generate_delta(&self, pool: &EntityPool) -> Option<crate::delta::Delta> {
        if !self.condition_met(pool) {
            return None;
        }

        let mut delta = crate::delta::Delta::new();

        match &self.effect {
            TendencyEffect::FillRole {
                role,
                source_kind,
                promoted_property,
            } => {
                // Find the first eligible alive entity to promote.
                let candidate_id = if let Some(kind) = source_kind {
                    pool.get_by_kind(kind)
                        .into_iter()
                        .find(|e| !e.roles.contains(role))
                        .map(|e| e.id)
                } else {
                    pool.all_entities()
                        .filter(|e| e.alive && !e.roles.contains(role))
                        .map(|e| e.id)
                        .next()
                };

                let cid = candidate_id?;

                let diff = delta.entity_diff_mut(cid);
                diff.role_diff.added.push(role.clone());
                if let Some((prop_name, prop_val)) = promoted_property {
                    diff.property_diff
                        .upserted
                        .insert(prop_name.clone(), prop_val.clone());
                }
            }

            TendencyEffect::SpawnEntity {
                kind,
                roles,
                initial_properties,
            } => {
                let mut entity = Entity::new(kind.as_str());
                entity.properties = initial_properties.clone();
                entity.roles = roles.iter().cloned().collect();
                delta.added_entities.insert(entity.id, entity);
            }

            TendencyEffect::EnsureEntityCount {
                kind,
                min_count,
                roles,
            } => {
                let current = pool.get_by_kind(kind).len();
                for _ in current..*min_count {
                    let mut entity = Entity::new(kind.as_str());
                    entity.roles = roles.iter().cloned().collect();
                    delta.added_entities.insert(entity.id, entity);
                }
            }

            TendencyEffect::TrimRole { role, keep_count } => {
                let occupants: Vec<_> = pool.get_by_role(role).into_iter().map(|e| e.id).collect();
                for &id in occupants.iter().skip(*keep_count) {
                    delta
                        .entity_diff_mut(id)
                        .role_diff
                        .removed
                        .push(role.clone());
                }
            }
        }

        if delta.is_empty() {
            None
        } else {
            Some(delta)
        }
    }
}
