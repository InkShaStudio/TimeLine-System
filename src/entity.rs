//! Entity model: individual world actors, their properties,
//! relationships and the pool that manages them.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::types::EntityId;

/// Maximum number of direct relationships a single entity may maintain.
///
/// Based on Dunbar's number (≈150 for humans), this constrains the social
/// graph density at the entity level while keeping the model realistic.
pub const DUNBAR_NUMBER: usize = 150;

/// Dynamic property value – a simple tagged union.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    List(Vec<Value>),
    Map(HashMap<String, Value>),
}

/// Key-value bag of entity attributes.
pub type Properties = HashMap<String, Value>;

/// A single actor in the world.
///
/// Entities are pure data objects; all state mutations happen through
/// [`crate::delta::Delta`] objects, keeping the system functional and
/// side-effect-free.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    /// Globally unique identifier.
    pub id: EntityId,
    /// Semantic category of the entity (e.g. `"hero"`, `"kingdom"`).
    pub kind: String,
    /// Arbitrary key/value attributes.
    pub properties: Properties,
    /// Logical roles this entity currently fills (e.g. `"unifier"`).
    /// Maintained by [`EntityPool`] so that role-vacancy queries are O(1).
    pub roles: HashSet<String>,
    /// Direct relationships to other entities.
    ///
    /// Capped at [`DUNBAR_NUMBER`]; attempts to exceed this limit return
    /// an error without panicking.
    pub relationships: Vec<EntityId>,
    /// `false` once the entity is deceased / deactivated.
    pub alive: bool,
}

impl Entity {
    /// Create a new entity of the given kind with a fresh random ID.
    pub fn new(kind: impl Into<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4(),
            kind: kind.into(),
            properties: HashMap::new(),
            roles: HashSet::new(),
            relationships: Vec::new(),
            alive: true,
        }
    }

    /// Create a new entity with an explicit ID (useful in deterministic tests).
    pub fn with_id(id: EntityId, kind: impl Into<String>) -> Self {
        Self {
            id,
            kind: kind.into(),
            properties: HashMap::new(),
            roles: HashSet::new(),
            relationships: Vec::new(),
            alive: true,
        }
    }

    /// Register a relationship to `other`, enforcing the Dunbar number.
    ///
    /// Duplicate entries are silently ignored.
    /// Returns `Err` when the Dunbar limit would be exceeded.
    pub fn add_relationship(&mut self, other: EntityId) -> Result<(), String> {
        if self.relationships.contains(&other) {
            return Ok(());
        }
        if self.relationships.len() >= DUNBAR_NUMBER {
            return Err(format!(
                "Entity {} has reached the Dunbar number limit ({}) for relationships",
                self.id, DUNBAR_NUMBER
            ));
        }
        self.relationships.push(other);
        Ok(())
    }

    /// Remove a relationship to `other` (no-op if not present).
    pub fn remove_relationship(&mut self, other: EntityId) {
        self.relationships.retain(|&id| id != other);
    }
}

/// Centralised store for all entities in the world.
///
/// Maintains an inverted role-index so that role-vacancy queries are O(1)
/// rather than O(n).  All mutations **must** go through the provided methods
/// to keep the index consistent.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EntityPool {
    entities: HashMap<EntityId, Entity>,
    /// role → set of entity IDs that currently fill that role (including dead ones).
    /// Alive filtering is done at query time.
    role_index: HashMap<String, HashSet<EntityId>>,
}

impl EntityPool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a new entity, indexing its roles.
    pub fn add_entity(&mut self, entity: Entity) {
        for role in &entity.roles {
            self.role_index
                .entry(role.clone())
                .or_default()
                .insert(entity.id);
        }
        self.entities.insert(entity.id, entity);
    }

    /// Remove an entity entirely and clean up its role index entries.
    pub fn remove_entity(&mut self, id: EntityId) -> Option<Entity> {
        if let Some(entity) = self.entities.remove(&id) {
            for role in &entity.roles {
                if let Some(set) = self.role_index.get_mut(role) {
                    set.remove(&id);
                }
            }
            Some(entity)
        } else {
            None
        }
    }

    pub fn get(&self, id: EntityId) -> Option<&Entity> {
        self.entities.get(&id)
    }

    pub fn get_mut(&mut self, id: EntityId) -> Option<&mut Entity> {
        self.entities.get_mut(&id)
    }

    /// Return all **alive** entities filling `role`.
    pub fn get_by_role(&self, role: &str) -> Vec<&Entity> {
        self.role_index
            .get(role)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.entities.get(id))
                    .filter(|e| e.alive)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Return all **alive** entities of the given `kind`.
    pub fn get_by_kind(&self, kind: &str) -> Vec<&Entity> {
        self.entities
            .values()
            .filter(|e| e.kind == kind && e.alive)
            .collect()
    }

    /// Iterate over every entity (alive or dead).
    pub fn all_entities(&self) -> impl Iterator<Item = &Entity> {
        self.entities.values()
    }

    /// `true` when no **alive** entity currently fills `role`.
    pub fn is_role_vacant(&self, role: &str) -> bool {
        self.get_by_role(role).is_empty()
    }

    /// Add or remove a role on an entity while keeping the role index in sync.
    ///
    /// This is the **only** correct way to modify roles; direct mutation of
    /// `entity.roles` bypasses the index.
    pub fn update_entity_role(&mut self, id: EntityId, role: String, add: bool) {
        if let Some(entity) = self.entities.get_mut(&id) {
            if add {
                entity.roles.insert(role.clone());
                self.role_index.entry(role).or_default().insert(id);
            } else {
                entity.roles.remove(&role);
                if let Some(set) = self.role_index.get_mut(&role) {
                    set.remove(&id);
                }
            }
        }
    }

    /// Apply a [`crate::delta::Delta`] to this pool in-place.
    ///
    /// This is the single mutation point used both during `advance` and during
    /// state reconstruction (timeline replay).
    pub fn apply_delta(&mut self, delta: &crate::delta::Delta) {
        // 1. Add brand-new entities (their roles are indexed via add_entity).
        for entity in delta.added_entities.values() {
            self.add_entity(entity.clone());
        }

        // 2. Patch existing entities.
        for (id, diff) in &delta.modified_entities {
            // --- property / relationship / alive patches (no index impact) ---
            if let Some(entity) = self.entities.get_mut(id) {
                for (key, value) in &diff.property_diff.upserted {
                    entity.properties.insert(key.clone(), value.clone());
                }
                for key in &diff.property_diff.removed {
                    entity.properties.remove(key);
                }
                for &rel_id in &diff.relationship_diff.added {
                    let _ = entity.add_relationship(rel_id);
                }
                for &rel_id in &diff.relationship_diff.removed {
                    entity.remove_relationship(rel_id);
                }
                if let Some(alive) = diff.alive_change {
                    entity.alive = alive;
                }
            }

            // --- role patches go through the index-aware helper ---
            for role in &diff.role_diff.added {
                self.update_entity_role(*id, role.clone(), true);
            }
            for role in &diff.role_diff.removed {
                self.update_entity_role(*id, role.clone(), false);
            }
        }

        // 3. Hard-remove entities.
        for id in &delta.removed_entities {
            self.remove_entity(*id);
        }
    }

    /// Number of entities currently in the pool (alive or dead).
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }
}
