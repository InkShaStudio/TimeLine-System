//! Causal propagation model.
//!
//! A [`CausalEvent`] records that something happened (the *cause*) and
//! specifies one or more [`EventEffect`]s that should materialise after an
//! optional delay.  The [`crate::manager::TimelineManager`] queues events
//! with a non-zero delay and "bakes" their effects into the delta of the step
//! when the delay expires.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::entity::Value;
use crate::types::{EntityId, EventId, NodeId};

/// A triggered causal event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CausalEvent {
    pub id: EventId,
    /// Human-readable category (e.g. `"battle"`, `"trade"`, `"plague"`).
    pub kind: String,
    pub description: String,
    /// What triggered this event.
    pub cause: EventSource,
    /// Effects that will be applied once `propagation_delay` steps elapse.
    pub effects: Vec<EventEffect>,
    /// How many `advance` calls must occur before effects are applied.
    /// `0` = apply immediately in the same step.
    pub propagation_delay: u64,
}

impl CausalEvent {
    /// Create an event that takes effect immediately (delay = 0).
    pub fn immediate(
        kind: impl Into<String>,
        description: impl Into<String>,
        cause: EventSource,
        effects: Vec<EventEffect>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            kind: kind.into(),
            description: description.into(),
            cause,
            effects,
            propagation_delay: 0,
        }
    }

    /// Create an event that takes effect after `delay` steps.
    pub fn delayed(
        kind: impl Into<String>,
        description: impl Into<String>,
        cause: EventSource,
        effects: Vec<EventEffect>,
        delay: u64,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            kind: kind.into(),
            description: description.into(),
            cause,
            effects,
            propagation_delay: delay,
        }
    }
}

/// What triggered the causal event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EventSource {
    /// An entity performed an action.
    Entity { id: EntityId, action: String },
    /// A system-level trigger (e.g. a tendency firing).
    System { description: String },
    /// Originated from a time-travel operation.
    TimeTravel { from_node: NodeId },
}

/// A single concrete change that a causal event produces when it matures.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EventEffect {
    KillEntity {
        id: EntityId,
    },
    SpawnEntity {
        kind: String,
        roles: Vec<String>,
        properties: HashMap<String, Value>,
    },
    ModifyProperty {
        entity_id: EntityId,
        property: String,
        value: Value,
    },
    AddRelationship {
        from: EntityId,
        to: EntityId,
    },
    RemoveRelationship {
        from: EntityId,
        to: EntityId,
    },
    AssignRole {
        entity_id: EntityId,
        role: String,
    },
    RemoveRole {
        entity_id: EntityId,
        role: String,
    },
}

/// Apply a single [`EventEffect`] into `delta`.
///
/// Exposed as a free function so both `TimelineManager` and tests can use it.
pub fn apply_effect_into_delta(delta: &mut crate::delta::Delta, effect: &EventEffect) {
    use crate::entity::Entity;

    match effect {
        EventEffect::KillEntity { id } => {
            delta.entity_diff_mut(*id).alive_change = Some(false);
        }
        EventEffect::SpawnEntity {
            kind,
            roles,
            properties,
        } => {
            let mut entity = Entity::new(kind.as_str());
            entity.properties = properties.clone();
            entity.roles = roles.iter().cloned().collect();
            delta.added_entities.insert(entity.id, entity);
        }
        EventEffect::ModifyProperty {
            entity_id,
            property,
            value,
        } => {
            delta
                .entity_diff_mut(*entity_id)
                .property_diff
                .upserted
                .insert(property.clone(), value.clone());
        }
        EventEffect::AddRelationship { from, to } => {
            delta
                .entity_diff_mut(*from)
                .relationship_diff
                .added
                .push(*to);
        }
        EventEffect::RemoveRelationship { from, to } => {
            delta
                .entity_diff_mut(*from)
                .relationship_diff
                .removed
                .push(*to);
        }
        EventEffect::AssignRole { entity_id, role } => {
            delta
                .entity_diff_mut(*entity_id)
                .role_diff
                .added
                .push(role.clone());
        }
        EventEffect::RemoveRole { entity_id, role } => {
            delta
                .entity_diff_mut(*entity_id)
                .role_diff
                .removed
                .push(role.clone());
        }
    }
}
