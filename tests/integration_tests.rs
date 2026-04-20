//! Integration tests for the timeline-system crate.
//!
//! Each test targets one requirement from the specification.

use timeline_system::prelude::*;

// ─────────────────────────────────────────────────────────────────────────────
// 1. Entity pool management
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn entity_pool_add_and_query() {
    let mut pool = EntityPool::new();
    let mut hero = Entity::new("hero");
    hero.roles.insert("leader".into());
    let hero_id = hero.id;

    pool.add_entity(hero);

    assert!(pool.get(hero_id).is_some());
    assert_eq!(pool.get_by_kind("hero").len(), 1);
    assert_eq!(pool.get_by_role("leader").len(), 1);
    assert!(!pool.is_role_vacant("leader"));
}

#[test]
fn entity_pool_remove_cleans_role_index() {
    let mut pool = EntityPool::new();
    let mut e = Entity::new("soldier");
    e.roles.insert("guard".into());
    let id = e.id;
    pool.add_entity(e);

    pool.remove_entity(id);

    assert!(pool.get(id).is_none());
    assert!(pool.is_role_vacant("guard"));
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. Dunbar number enforcement
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn dunbar_number_enforced() {
    let mut entity = Entity::new("human");
    for _ in 0..DUNBAR_NUMBER {
        let other_id = uuid::Uuid::new_v4();
        assert!(entity.add_relationship(other_id).is_ok());
    }
    // One over the limit must fail.
    let extra_id = uuid::Uuid::new_v4();
    assert!(entity.add_relationship(extra_id).is_err());
    assert_eq!(entity.relationships.len(), DUNBAR_NUMBER);
}

#[test]
fn dunbar_duplicate_is_idempotent() {
    let mut entity = Entity::new("human");
    let other_id = uuid::Uuid::new_v4();
    entity.add_relationship(other_id).unwrap();
    entity.add_relationship(other_id).unwrap(); // duplicate — should not grow
    assert_eq!(entity.relationships.len(), 1);
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. Delta application (incremental storage)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn delta_add_entity() {
    let mut mgr = TimelineManager::new();
    let hero = Entity::new("hero");
    let id = hero.id;

    mgr.advance(Delta::new().add_entity(hero), "spawn hero");

    assert!(mgr.entity_pool().get(id).is_some());
}

#[test]
fn delta_kill_entity() {
    let mut mgr = TimelineManager::new();
    let e = Entity::new("npc");
    let id = e.id;
    mgr.advance(Delta::new().add_entity(e), "spawn npc");
    mgr.advance(Delta::new().kill_entity(id), "kill npc");

    assert_eq!(mgr.entity_pool().get(id).unwrap().alive, false);
    // Dead entities are not returned by get_by_kind.
    assert!(mgr.entity_pool().get_by_kind("npc").is_empty());
}

#[test]
fn delta_set_property() {
    let mut mgr = TimelineManager::new();
    let e = Entity::new("city");
    let id = e.id;
    mgr.advance(Delta::new().add_entity(e), "found city");
    mgr.advance(
        Delta::new().set_property(id, "population", Value::Int(1000)),
        "grow population",
    );

    let city = mgr.entity_pool().get(id).unwrap();
    assert_eq!(city.properties["population"], Value::Int(1000));
}

// ─────────────────────────────────────────────────────────────────────────────
// 4. Functional / pure-snapshot: old state is preserved
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn past_state_is_immutable() {
    let mut mgr = TimelineManager::new();
    let e = Entity::new("hero");
    let id = e.id;
    let node1 = mgr.advance(Delta::new().add_entity(e), "create hero");
    let _node2 = mgr.advance(
        Delta::new().set_property(id, "level", Value::Int(10)),
        "level up",
    );

    // Reconstruct state at node1 — hero should have no "level" property.
    let past = mgr.reconstruct_state_at(node1);
    assert!(!past.get(id).unwrap().properties.contains_key("level"));
}

// ─────────────────────────────────────────────────────────────────────────────
// 5. Stable linear forward time
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn timestamps_are_monotonically_increasing() {
    let mut mgr = TimelineManager::new();
    let n1 = mgr.advance(Delta::new(), "t1");
    let n2 = mgr.advance(Delta::new(), "t2");
    let n3 = mgr.advance(Delta::new(), "t3");

    let ts = |id| mgr.get_node(id).unwrap().timestamp;
    assert!(ts(n1) < ts(n2));
    assert!(ts(n2) < ts(n3));
}

// ─────────────────────────────────────────────────────────────────────────────
// 6. Timeline branching
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn branch_creates_independent_timeline() {
    let mut mgr = TimelineManager::new();
    let e = Entity::new("kingdom");
    let id = e.id;
    let node1 = mgr.advance(Delta::new().add_entity(e), "found kingdom");

    // Branch from node1.
    let main_id = mgr.active_timeline_id();
    let branch_id = mgr.branch_at(node1, "AlternateBranch").unwrap();

    // Advance on the branch.
    mgr.advance(
        Delta::new().set_property(id, "ruler", Value::Text("Branch Ruler".into())),
        "install branch ruler",
    );
    let branch_kingdom = mgr.entity_pool().get(id).unwrap().clone();

    // Switch back to main and verify it is unaffected.
    mgr.switch_timeline(main_id).unwrap();
    let main_kingdom = mgr.entity_pool().get(id).unwrap().clone();

    assert_eq!(
        branch_kingdom.properties.get("ruler"),
        Some(&Value::Text("Branch Ruler".into()))
    );
    assert!(!main_kingdom.properties.contains_key("ruler"));
    assert_ne!(main_id, branch_id);
}

// ─────────────────────────────────────────────────────────────────────────────
// 7. Time travel (grandfather paradox free)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn time_travel_does_not_alter_original_timeline() {
    let mut mgr = TimelineManager::new();
    let n1 = mgr.advance(Delta::new(), "step 1");
    let n2 = mgr.advance(Delta::new(), "step 2");
    let n3 = mgr.advance(Delta::new(), "step 3");
    let original_id = mgr.active_timeline_id();

    // Travel back to n1 — creates a new branch.
    let travel_branch_id = mgr.time_travel_to(n1).unwrap();
    assert!(mgr.get_timeline(travel_branch_id).unwrap().is_time_travel_branch);
    assert_ne!(travel_branch_id, original_id);

    // The original timeline still has all three nodes.
    let orig = mgr.get_timeline(original_id).unwrap();
    assert!(orig.nodes.contains(&n2));
    assert!(orig.nodes.contains(&n3));
}

#[test]
fn time_travel_reconstructs_correct_state() {
    let mut mgr = TimelineManager::new();
    let e = Entity::new("hero");
    let id = e.id;
    let node1 = mgr.advance(Delta::new().add_entity(e), "spawn hero");
    mgr.advance(
        Delta::new().set_property(id, "power", Value::Int(50)),
        "power up",
    );

    // Travel back to node1 — hero should have no "power" property.
    mgr.time_travel_to(node1).unwrap();
    assert!(!mgr.entity_pool().get(id).unwrap().properties.contains_key("power"));
}

// ─────────────────────────────────────────────────────────────────────────────
// 8. World tendencies & self-correction
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn tendency_fills_vacant_role_automatically() {
    let mut mgr = TimelineManager::new();

    mgr.add_tendency(Tendency::new(
        "UnificationTendency",
        "The realm must have a unifier",
        TendencyCondition::RoleVacant { role: "unifier".into() },
        TendencyEffect::FillRole {
            role: "unifier".into(),
            source_kind: Some("hero".into()),
            promoted_property: Some(("title".into(), Value::Text("Grand Unifier".into()))),
        },
    ));

    // Spawn Zhang San and give him the unifier role.
    let zhang_san = Entity::new("hero");
    let zs_id = zhang_san.id;
    mgr.advance(
        Delta::new()
            .add_entity(zhang_san)
            .assign_role(zs_id, "unifier"),
        "Zhang San rises as unifier",
    );

    // Spawn Li Si as a successor candidate.
    let li_si = Entity::new("hero");
    mgr.advance(Delta::new().add_entity(li_si), "Li Si appears");

    // Kill Zhang San.
    mgr.advance(Delta::new().kill_entity(zs_id), "Zhang San dies");

    // Tendency must have auto-promoted Li Si.
    let unifiers = mgr.entity_pool().get_by_role("unifier");
    assert!(!unifiers.is_empty(), "Unifier role must be filled after Zhang San dies");
    assert!(
        unifiers.iter().all(|e| e.alive),
        "The new unifier must be alive"
    );
}

#[test]
fn tendency_spawns_entity_when_count_too_low() {
    let mut mgr = TimelineManager::new();

    mgr.add_tendency(Tendency::new(
        "MinimumGuards",
        "There must be at least 3 guards",
        TendencyCondition::EntityCountBelow {
            kind: "guard".into(),
            threshold: 3,
        },
        TendencyEffect::EnsureEntityCount {
            kind: "guard".into(),
            min_count: 3,
            roles: vec!["guard".into()],
        },
    ));

    // No guards yet — the tendency should spawn 3 when we advance.
    mgr.advance(Delta::new(), "world tick");

    let guards = mgr.entity_pool().get_by_kind("guard");
    assert_eq!(guards.len(), 3, "3 guards should have been spawned");
}

#[test]
fn tendency_with_priority_fires_first() {
    let mut mgr = TimelineManager::new();

    // Low-priority: spawn a "warrior".
    mgr.add_tendency(
        Tendency::new(
            "SpawnWarrior",
            "",
            TendencyCondition::Always,
            TendencyEffect::SpawnEntity {
                kind: "warrior".into(),
                roles: vec![],
                initial_properties: Default::default(),
            },
        )
        .with_priority(0),
    );

    // High-priority: spawn a "king" — should appear before warrior in effects.
    mgr.add_tendency(
        Tendency::new(
            "SpawnKing",
            "",
            TendencyCondition::Always,
            TendencyEffect::SpawnEntity {
                kind: "king".into(),
                roles: vec![],
                initial_properties: Default::default(),
            },
        )
        .with_priority(10),
    );

    mgr.advance(Delta::new(), "tick");

    assert_eq!(mgr.entity_pool().get_by_kind("king").len(), 1);
    assert_eq!(mgr.entity_pool().get_by_kind("warrior").len(), 1);
}

// ─────────────────────────────────────────────────────────────────────────────
// 9. Causal propagation
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn immediate_causal_event_applies_in_same_step() {
    let mut mgr = TimelineManager::new();
    let victim = Entity::new("npc");
    let vid = victim.id;
    mgr.advance(Delta::new().add_entity(victim), "spawn npc");

    let event = CausalEvent::immediate(
        "assassination",
        "NPC is killed immediately",
        EventSource::System {
            description: "plot".into(),
        },
        vec![EventEffect::KillEntity { id: vid }],
    );

    mgr.advance(Delta::new().with_event(event), "trigger assassination");

    assert_eq!(mgr.entity_pool().get(vid).unwrap().alive, false);
}

#[test]
fn delayed_causal_event_applies_after_delay() {
    let mut mgr = TimelineManager::new();
    let target = Entity::new("city");
    let tid = target.id;
    mgr.advance(Delta::new().add_entity(target), "found city");

    // delay=2 means the effect applies exactly 2 `advance` calls after the
    // call that queued the event.  The queuing happens inside "trigger plague"
    // below, so: tick 2 = 1st subsequent call (not fired), plague lands = 2nd.
    let event = CausalEvent::delayed(
        "plague",
        "Plague arrives 2 advance steps after the trigger",
        EventSource::System {
            description: "nature".into(),
        },
        vec![EventEffect::ModifyProperty {
            entity_id: tid,
            property: "plague_struck".into(),
            value: Value::Bool(true),
        }],
        2,
    );

    mgr.advance(Delta::new().with_event(event), "trigger plague");
    // 0 of 2 subsequent advances completed — not fired yet.
    assert!(
        !mgr
            .entity_pool()
            .get(tid)
            .unwrap()
            .properties
            .contains_key("plague_struck")
    );

    mgr.advance(Delta::new(), "tick 2");
    // 1 of 2 subsequent advances completed — still not fired.
    assert!(
        !mgr
            .entity_pool()
            .get(tid)
            .unwrap()
            .properties
            .contains_key("plague_struck")
    );

    // 2nd subsequent advance after trigger — delay counter reaches 0, plague lands.
    mgr.advance(Delta::new(), "tick 3 — plague lands");
    assert_eq!(
        mgr.entity_pool()
            .get(tid)
            .unwrap()
            .properties
            .get("plague_struck"),
        Some(&Value::Bool(true))
    );
}

#[test]
fn time_travel_clears_pending_events() {
    let mut mgr = TimelineManager::new();
    let target = Entity::new("city");
    let tid = target.id;
    let node0 = mgr.advance(Delta::new().add_entity(target), "found city");

    let event = CausalEvent::delayed(
        "plague",
        "Delayed plague",
        EventSource::System {
            description: "".into(),
        },
        vec![EventEffect::ModifyProperty {
            entity_id: tid,
            property: "plagued".into(),
            value: Value::Bool(true),
        }],
        3,
    );
    mgr.advance(Delta::new().with_event(event), "trigger plague");

    // Time-travel to node0 — pending events must be cleared.
    mgr.time_travel_to(node0).unwrap();
    assert_eq!(mgr.pending_event_count(), 0);

    // Advance several steps — plague must never land.
    for _ in 0..5 {
        mgr.advance(Delta::new(), "tick");
    }
    assert!(
        !mgr
            .entity_pool()
            .get(tid)
            .unwrap()
            .properties
            .contains_key("plagued")
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 10. Full history / state reconstruction
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn full_history_includes_all_ancestor_nodes() {
    let mut mgr = TimelineManager::new();
    let n1 = mgr.advance(Delta::new(), "step 1");
    let n2 = mgr.advance(Delta::new(), "step 2");
    let n3 = mgr.advance(Delta::new(), "step 3");

    let history: Vec<NodeId> = mgr.full_history().iter().map(|n| n.id).collect();
    assert!(history.contains(&n1));
    assert!(history.contains(&n2));
    assert!(history.contains(&n3));
}

#[test]
fn reconstruct_state_at_arbitrary_node() {
    let mut mgr = TimelineManager::new();
    let e = Entity::new("empire");
    let eid = e.id;
    let n1 = mgr.advance(Delta::new().add_entity(e), "found empire");
    let _n2 = mgr.advance(
        Delta::new().set_property(eid, "size", Value::Int(1)),
        "grow",
    );
    let _n3 = mgr.advance(
        Delta::new().set_property(eid, "size", Value::Int(2)),
        "grow more",
    );

    let state_at_n1 = mgr.reconstruct_state_at(n1);
    assert!(
        !state_at_n1
            .get(eid)
            .unwrap()
            .properties
            .contains_key("size")
    );

    let current = mgr.entity_pool().get(eid).unwrap();
    assert_eq!(current.properties["size"], Value::Int(2));
}

// ─────────────────────────────────────────────────────────────────────────────
// 11. Serde round-trip (embeddability: the types must be serialisable)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn entity_serde_round_trip() {
    let mut e = Entity::new("hero");
    e.properties
        .insert("name".into(), Value::Text("Zhang San".into()));
    e.roles.insert("unifier".into());

    let json = serde_json::to_string(&e).expect("serialise");
    let back: Entity = serde_json::from_str(&json).expect("deserialise");

    assert_eq!(e.id, back.id);
    assert_eq!(e.kind, back.kind);
    assert_eq!(e.properties["name"], back.properties["name"]);
    assert!(back.roles.contains("unifier"));
}

#[test]
fn delta_serde_round_trip() {
    let e = Entity::new("city");
    let eid = e.id;
    let delta = Delta::new()
        .add_entity(e)
        .set_property(eid, "pop", Value::Int(500));

    let json = serde_json::to_string(&delta).expect("serialise");
    let back: Delta = serde_json::from_str(&json).expect("deserialise");

    assert!(back.added_entities.contains_key(&eid));
    assert!(back.modified_entities.contains_key(&eid));
}

// ─────────────────────────────────────────────────────────────────────────────
// 12. Relationship management via Delta
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn relationship_added_and_removed_via_delta() {
    let mut mgr = TimelineManager::new();
    let a = Entity::new("person");
    let b = Entity::new("person");
    let (aid, bid) = (a.id, b.id);

    mgr.advance(
        Delta::new().add_entity(a).add_entity(b),
        "spawn two people",
    );
    mgr.advance(
        Delta::new().add_relationship(aid, bid),
        "they meet",
    );

    assert!(mgr.entity_pool().get(aid).unwrap().relationships.contains(&bid));

    // Remove the relationship.
    let mut remove_delta = Delta::new();
    remove_delta.entity_diff_mut(aid).relationship_diff.removed.push(bid);
    mgr.advance(remove_delta, "they part ways");

    assert!(!mgr.entity_pool().get(aid).unwrap().relationships.contains(&bid));
}
