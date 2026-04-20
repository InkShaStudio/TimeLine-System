/// Shared type aliases for all identifiers in the timeline system.
///
/// Using `Uuid` as a base ensures globally unique IDs across timelines,
/// branches, and entity pools.
use uuid::Uuid;

/// Unique identifier for an entity in the entity pool.
pub type EntityId = Uuid;

/// Unique identifier for a single timeline node (snapshot).
pub type NodeId = Uuid;

/// Unique identifier for a timeline (main or branch).
pub type TimelineId = Uuid;

/// Unique identifier for a causal event.
pub type EventId = Uuid;

/// Unique identifier for a world tendency.
pub type TendencyId = Uuid;
