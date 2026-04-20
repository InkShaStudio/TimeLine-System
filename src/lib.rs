//! # Timeline System
//!
//! A lightweight, embeddable, functional timeline engine written in Rust.
//!
//! ## Design highlights
//!
//! | Property | Implementation |
//! |---|---|
//! | Lightweight / embeddable | Pure library crate; zero runtime threads |
//! | Stable linear forward time | Each [`advance`][manager::TimelineManager::advance] call appends one immutable node |
//! | Branchable timelines | [`branch_at`][manager::TimelineManager::branch_at] forks any node into a child timeline |
//! | Pure, side-effect-free | All mutations are expressed as [`Delta`][delta::Delta] objects; no global state |
//! | Functional snapshots | Every node is a new snapshot; old nodes are never mutated |
//! | Incremental storage | Nodes store only the *diff* from their parent (delta compression) |
//! | Time travel (no grandfather paradox) | [`time_travel_to`][manager::TimelineManager::time_travel_to] creates a fresh branch; origin is untouched |
//! | Self-correcting tendencies | World [`Tendency`][tendency::Tendency] objects auto-correct after disturbances |
//! | Entity pool | [`EntityPool`][entity::EntityPool] is the single source of truth for all entities |
//! | Causal propagation | [`CausalEvent`][causal::CausalEvent]s are queued with an optional delay |
//! | Dunbar number | Each entity's relationship list is capped at [`DUNBAR_NUMBER`][entity::DUNBAR_NUMBER] (150) |
//!
//! ## Quick start
//!
//! ```rust
//! use timeline_system::prelude::*;
//!
//! let mut mgr = TimelineManager::new();
//!
//! // Register a self-correcting world tendency.
//! mgr.add_tendency(Tendency::new(
//!     "UnificationTendency",
//!     "The realm must always have a unifier",
//!     TendencyCondition::RoleVacant { role: "unifier".into() },
//!     TendencyEffect::FillRole {
//!         role: "unifier".into(),
//!         source_kind: Some("hero".into()),
//!         promoted_property: Some(("title".into(), Value::Text("Grand Unifier".into()))),
//!     },
//! ));
//!
//! // Create Zhang San and give him the "unifier" role.
//! let zhang_san = Entity::new("hero");
//! let zs_id = zhang_san.id;
//! let _node1 = mgr.advance(
//!     Delta::new()
//!         .add_entity(zhang_san)
//!         .assign_role(zs_id, "unifier"),
//!     "Zhang San rises as the unifier",
//! );
//!
//! // Create Li Si (a candidate to succeed Zhang San).
//! let li_si = Entity::new("hero");
//! let _node2 = mgr.advance(
//!     Delta::new().add_entity(li_si),
//!     "Li Si appears",
//! );
//!
//! // Kill Zhang San — the tendency auto-promotes Li Si.
//! let _node3 = mgr.advance(
//!     Delta::new().kill_entity(zs_id),
//!     "Zhang San dies in battle",
//! );
//!
//! assert!(
//!     !mgr.entity_pool().get_by_role("unifier").is_empty(),
//!     "Li Si should have been promoted automatically"
//! );
//! ```

pub mod causal;
pub mod delta;
pub mod entity;
pub mod manager;
pub mod node;
pub mod tendency;
pub mod timeline;
pub mod types;

/// Convenience re-exports for the most common types.
pub mod prelude {
    pub use crate::causal::{CausalEvent, EventEffect, EventSource};
    pub use crate::delta::{Delta, EntityDiff, PropertyDiff, RelationshipDiff, RoleDiff};
    pub use crate::entity::{Entity, EntityPool, Value, DUNBAR_NUMBER};
    pub use crate::manager::TimelineManager;
    pub use crate::node::TimelineNode;
    pub use crate::tendency::{Tendency, TendencyCondition, TendencyEffect};
    pub use crate::timeline::Timeline;
    pub use crate::types::{EntityId, EventId, NodeId, TendencyId, TimelineId};
}
