//! Containers and containment (Blueprint §4.4).
//!
//! Invariants, checked by [`validate_containment`] (run after every mutation in debug builds and on
//! load):
//!
//! * every object has exactly one location, so at most one parent;
//! * there are no containment cycles;
//! * a child satisfies its container's `accepts` rule and the container's capacity;
//! * parent and child agree: the child's location names the container, and the container lists the child;
//! * instances have exactly the container slots their template defines.
//!
//! Deleting a container owner follows a declared policy: [`DeletePolicy::Cascade`] deletes the children,
//! [`DeletePolicy::Evict`] moves them to the owner's place, [`DeletePolicy::Forbid`] refuses. Every
//! mutation is all-or-nothing.

use crate::id::{EntityId, IdsExhausted, Kind};
use crate::map::Tile;
use crate::object::{ContainerState, Location, ObjectInstance, Parent};
use crate::world::WorldState;
use pg_content::{ContentSet, ResolvedTemplate, TemplateId, ValidationReport};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// What a container will take: matches if it has any included tag or is a listed template (or the
/// include lists are empty), and never if it has an excluded tag.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AcceptRule {
    pub tags: Vec<String>,
    pub exclude_tags: Vec<String>,
    pub templates: Vec<String>,
}

impl AcceptRule {
    pub fn accepts(&self, child: &ResolvedTemplate) -> bool {
        if self.exclude_tags.iter().any(|t| child.has_tag(t)) {
            return false;
        }
        if self.tags.is_empty() && self.templates.is_empty() {
            return true;
        }
        self.tags.iter().any(|t| child.has_tag(t))
            || self.templates.iter().any(|t| t == child.id().as_str())
    }
}

/// One container definition read from a resolved template's `container` component.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerDef {
    pub id: String,
    pub capacity: usize,
    pub ordered: bool,
    pub accepts: AcceptRule,
}

fn strings(c: Option<&pg_canon::Canon>) -> Vec<String> {
    c.and_then(pg_canon::Canon::as_list)
        .map(|l| {
            l.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// The container definitions of a resolved template (already validated and defaulted by the content system).
pub fn container_defs(t: &ResolvedTemplate) -> Vec<ContainerDef> {
    let Some(list) = t
        .component("container")
        .and_then(|c| c.get("containers"))
        .and_then(pg_canon::Canon::as_list)
    else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|c| {
            let accepts = c.get("accepts");
            Some(ContainerDef {
                id: c.get("id")?.as_str()?.to_owned(),
                capacity: usize::try_from(c.get("capacity")?.as_u64()?).ok()?,
                ordered: c
                    .get("ordered")
                    .and_then(pg_canon::Canon::as_bool)
                    .unwrap_or(false),
                accepts: AcceptRule {
                    tags: strings(accepts.and_then(|a| a.get("tags"))),
                    exclude_tags: strings(accepts.and_then(|a| a.get("exclude_tags"))),
                    templates: strings(accepts.and_then(|a| a.get("templates"))),
                },
            })
        })
        .collect()
}

/// What to do with an object's contents when it is deleted.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DeletePolicy {
    Cascade,
    Evict,
    Forbid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContainmentError {
    UnknownObject(EntityId),
    UnknownTemplate(TemplateId),
    UnknownMap(EntityId),
    OutOfBounds(Tile),
    UnknownContainer {
        owner: EntityId,
        container: String,
    },
    NotAccepted {
        child: EntityId,
        owner: EntityId,
        container: String,
    },
    Full {
        owner: EntityId,
        container: String,
    },
    /// Putting an object inside itself or inside one of its own descendants.
    Cycle {
        child: EntityId,
        owner: EntityId,
    },
    HasChildren(EntityId),
    Ids(IdsExhausted),
}

impl fmt::Display for ContainmentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContainmentError::UnknownObject(o) => write!(f, "no object {o}"),
            ContainmentError::UnknownTemplate(t) => write!(f, "no template '{t}'"),
            ContainmentError::UnknownMap(m) => write!(f, "no map {m}"),
            ContainmentError::OutOfBounds(t) => write!(f, "tile {t} is outside the map"),
            ContainmentError::UnknownContainer { owner, container } => {
                write!(f, "{owner} has no container '{container}'")
            }
            ContainmentError::NotAccepted {
                child,
                owner,
                container,
            } => {
                write!(f, "{owner}.{container} does not accept {child}")
            }
            ContainmentError::Full { owner, container } => write!(f, "{owner}.{container} is full"),
            ContainmentError::Cycle { child, owner } => {
                write!(f, "{child} cannot go inside {owner}: that would be a cycle")
            }
            ContainmentError::HasChildren(o) => {
                write!(f, "{o} still holds objects and its delete policy is forbid")
            }
            ContainmentError::Ids(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for ContainmentError {}

impl From<IdsExhausted> for ContainmentError {
    fn from(e: IdsExhausted) -> Self {
        ContainmentError::Ids(e)
    }
}

fn template_of<'a>(
    world: &WorldState,
    content: &'a ContentSet,
    id: EntityId,
) -> Result<&'a std::sync::Arc<ResolvedTemplate>, ContainmentError> {
    let obj = world
        .objects
        .get(id)
        .ok_or(ContainmentError::UnknownObject(id))?;
    content
        .get(&obj.template)
        .ok_or_else(|| ContainmentError::UnknownTemplate(obj.template.clone()))
}

/// Whether `id` is `ancestor` or sits (at any depth) inside it. Bounded so corrupt data cannot loop.
fn is_inside(world: &WorldState, id: EntityId, ancestor: EntityId) -> bool {
    let mut current = id;
    for _ in 0..=world.objects.len() {
        if current == ancestor {
            return true;
        }
        match world.objects.get(current).map(|o| &o.location) {
            Some(Location::InContainer(p)) => current = p.owner,
            _ => return false,
        }
    }
    true // a chain longer than the table is itself a cycle
}

/// Checks that `child` (an existing object) may be added to `owner.container`, ignoring whether it is
/// currently somewhere else.
fn check_insert(
    world: &WorldState,
    content: &ContentSet,
    child: EntityId,
    child_template: &ResolvedTemplate,
    owner: EntityId,
    container: &str,
    already_inside: bool,
) -> Result<(), ContainmentError> {
    let owner_obj = world
        .objects
        .get(owner)
        .ok_or(ContainmentError::UnknownObject(owner))?;
    let owner_template = content
        .get(&owner_obj.template)
        .ok_or_else(|| ContainmentError::UnknownTemplate(owner_obj.template.clone()))?;
    let def = container_defs(owner_template)
        .into_iter()
        .find(|d| d.id == container)
        .ok_or_else(|| ContainmentError::UnknownContainer {
            owner,
            container: container.to_owned(),
        })?;
    if child == owner || is_inside(world, owner, child) {
        return Err(ContainmentError::Cycle { child, owner });
    }
    if !def.accepts.accepts(child_template) {
        return Err(ContainmentError::NotAccepted {
            child,
            owner,
            container: container.to_owned(),
        });
    }
    let used = owner_obj
        .containers
        .get(container)
        .map_or(0, |c| c.slots.len());
    let used = if already_inside {
        used.saturating_sub(1)
    } else {
        used
    };
    if used >= def.capacity {
        return Err(ContainmentError::Full {
            owner,
            container: container.to_owned(),
        });
    }
    Ok(())
}

fn debug_check(world: &WorldState, content: &ContentSet) {
    if cfg!(debug_assertions) {
        let report = validate_containment(world, content);
        debug_assert!(report.is_ok(), "containment invariants broken:\n{report}");
    }
}

/// Creates an object from `template` at `location` and returns its id. All-or-nothing; a refused spawn
/// does not consume an id.
pub fn spawn_object(
    world: &mut WorldState,
    content: &ContentSet,
    template: &TemplateId,
    location: Location,
) -> Result<EntityId, ContainmentError> {
    let resolved = content
        .get(template)
        .ok_or_else(|| ContainmentError::UnknownTemplate(template.clone()))?;
    match &location {
        Location::OnMap { map, tile } => {
            let m = world
                .maps
                .get(*map)
                .ok_or(ContainmentError::UnknownMap(*map))?;
            if !m.in_bounds(*tile) {
                return Err(ContainmentError::OutOfBounds(*tile));
            }
        }
        Location::InContainer(p) => {
            // The id is not allocated yet; use a placeholder that cannot clash with a real object for the
            // cycle check (a new object has no descendants, so only the accept/capacity checks matter).
            let probe = EntityId::new(Kind::Object, 0);
            check_insert(
                world,
                content,
                probe,
                resolved,
                p.owner,
                &p.container,
                false,
            )?;
        }
    }
    let id = world.id_counters.allocate(Kind::Object)?;
    let containers: BTreeMap<String, ContainerState> = container_defs(resolved)
        .into_iter()
        .map(|d| (d.id, ContainerState::default()))
        .collect();
    if let Location::InContainer(p) = &location {
        if let Some(c) = world
            .objects
            .get_mut(p.owner)
            .and_then(|o| o.containers.get_mut(&p.container))
        {
            c.slots.push(id);
        }
    }
    let _ = world.objects.insert(
        id,
        ObjectInstance {
            id,
            template: template.clone(),
            location,
            containers,
        },
    );
    debug_check(world, content);
    Ok(id)
}

/// Removes `id` from the container list it currently sits in (if any). Callers validate first, so this
/// never needs to be undone.
fn detach(world: &mut WorldState, id: EntityId) {
    let Some(Location::InContainer(p)) = world.objects.get(id).map(|o| o.location.clone()) else {
        return;
    };
    if let Some(c) = world
        .objects
        .get_mut(p.owner)
        .and_then(|o| o.containers.get_mut(&p.container))
    {
        c.slots.retain(|s| *s != id);
    }
}

/// Puts an object on a map tile (taking it out of any container).
pub fn move_to_tile(
    world: &mut WorldState,
    content: &ContentSet,
    id: EntityId,
    map: EntityId,
    tile: Tile,
) -> Result<(), ContainmentError> {
    if !world.objects.contains(id) {
        return Err(ContainmentError::UnknownObject(id));
    }
    let m = world
        .maps
        .get(map)
        .ok_or(ContainmentError::UnknownMap(map))?;
    if !m.in_bounds(tile) {
        return Err(ContainmentError::OutOfBounds(tile));
    }
    detach(world, id);
    if let Some(o) = world.objects.get_mut(id) {
        o.location = Location::OnMap { map, tile };
    }
    debug_check(world, content);
    Ok(())
}

/// Moves an object into `owner.container`, from the map or from another container.
pub fn move_into(
    world: &mut WorldState,
    content: &ContentSet,
    child: EntityId,
    owner: EntityId,
    container: &str,
) -> Result<(), ContainmentError> {
    let child_template = template_of(world, content, child)?.clone();
    let already_inside = matches!(
        world.objects.get(child).map(|o| &o.location),
        Some(Location::InContainer(p)) if p.owner == owner && p.container == container
    );
    check_insert(
        world,
        content,
        child,
        &child_template,
        owner,
        container,
        already_inside,
    )?;
    detach(world, child);
    if let Some(c) = world
        .objects
        .get_mut(owner)
        .and_then(|o| o.containers.get_mut(container))
    {
        c.slots.push(child);
    }
    if let Some(o) = world.objects.get_mut(child) {
        o.location = Location::InContainer(Parent {
            owner,
            container: container.to_owned(),
        });
    }
    debug_check(world, content);
    Ok(())
}

/// Every object inside `id` at any depth, depth-first in container-id then slot order.
pub fn descendants(world: &WorldState, id: EntityId) -> Vec<EntityId> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    seen.insert(id);
    let mut stack = vec![id];
    while let Some(current) = stack.pop() {
        let Some(obj) = world.objects.get(current) else {
            continue;
        };
        let mut kids = Vec::new();
        for state in obj.containers.values() {
            kids.extend(state.slots.iter().copied());
        }
        // Push in reverse so the first child is visited first.
        for kid in kids.into_iter().rev() {
            if seen.insert(kid) {
                stack.push(kid);
            }
        }
        if current != id {
            out.push(current);
        }
    }
    out
}

/// Deletes an object under `policy`. Returns every id that was removed. All-or-nothing.
pub fn delete_object(
    world: &mut WorldState,
    content: &ContentSet,
    id: EntityId,
    policy: DeletePolicy,
) -> Result<Vec<EntityId>, ContainmentError> {
    let obj = world
        .objects
        .get(id)
        .ok_or(ContainmentError::UnknownObject(id))?;
    let location = obj.location.clone();
    let direct: Vec<EntityId> = obj
        .containers
        .values()
        .flat_map(|c| c.slots.iter().copied())
        .collect();

    let mut removed = vec![id];
    match policy {
        DeletePolicy::Forbid => {
            if !direct.is_empty() {
                return Err(ContainmentError::HasChildren(id));
            }
        }
        DeletePolicy::Cascade => {
            removed.extend(descendants(world, id));
        }
        DeletePolicy::Evict => {
            // Pre-check that every child fits in the deleted object's place.
            if let Location::InContainer(p) = &location {
                let owner_obj = world
                    .objects
                    .get(p.owner)
                    .ok_or(ContainmentError::UnknownObject(p.owner))?;
                let owner_template = content
                    .get(&owner_obj.template)
                    .ok_or_else(|| ContainmentError::UnknownTemplate(owner_obj.template.clone()))?;
                let def = container_defs(owner_template)
                    .into_iter()
                    .find(|d| d.id == p.container)
                    .ok_or_else(|| ContainmentError::UnknownContainer {
                        owner: p.owner,
                        container: p.container.clone(),
                    })?;
                let used = owner_obj
                    .containers
                    .get(&p.container)
                    .map_or(0, |c| c.slots.len());
                if used.saturating_sub(1) + direct.len() > def.capacity {
                    return Err(ContainmentError::Full {
                        owner: p.owner,
                        container: p.container.clone(),
                    });
                }
                for kid in &direct {
                    let kt = template_of(world, content, *kid)?;
                    if !def.accepts.accepts(kt) {
                        return Err(ContainmentError::NotAccepted {
                            child: *kid,
                            owner: p.owner,
                            container: p.container.clone(),
                        });
                    }
                }
            }
        }
    }

    // Apply.
    detach(world, id);
    if policy == DeletePolicy::Evict {
        for kid in &direct {
            match &location {
                Location::OnMap { map, tile } => {
                    if let Some(o) = world.objects.get_mut(*kid) {
                        o.location = Location::OnMap {
                            map: *map,
                            tile: *tile,
                        };
                    }
                }
                Location::InContainer(p) => {
                    if let Some(c) = world
                        .objects
                        .get_mut(p.owner)
                        .and_then(|o| o.containers.get_mut(&p.container))
                    {
                        c.slots.push(*kid);
                    }
                    if let Some(o) = world.objects.get_mut(*kid) {
                        o.location = Location::InContainer(p.clone());
                    }
                }
            }
        }
    }
    for gone in &removed {
        world.objects.remove(*gone);
    }
    debug_check(world, content);
    Ok(removed)
}

/// Checks every containment invariant and reports each violation (it does not stop at the first).
pub fn validate_containment(world: &WorldState, content: &ContentSet) -> ValidationReport {
    let mut report = ValidationReport::new();
    // Which container lists each object, to detect duplicates and parent/child disagreement.
    let mut listed_in: BTreeMap<EntityId, Vec<(EntityId, String)>> = BTreeMap::new();

    for (id, obj) in world.objects.iter() {
        let path = id.to_string();
        let Some(resolved) = content.get(&obj.template) else {
            report.error(
                "unknown_template",
                &path,
                format!("object uses unknown template '{}'", obj.template),
            );
            continue;
        };
        let defs = container_defs(resolved);

        // The instance has exactly the template's containers.
        for d in &defs {
            if !obj.containers.contains_key(&d.id) {
                report.error(
                    "missing_container_state",
                    format!("{path}.containers.{}", d.id),
                    "template container has no instance state",
                );
            }
        }
        for (cid, state) in &obj.containers {
            let cpath = format!("{path}.containers.{cid}");
            let Some(def) = defs.iter().find(|d| &d.id == cid) else {
                report.error(
                    "extra_container_state",
                    &cpath,
                    "instance has a container its template does not define",
                );
                continue;
            };
            if state.slots.len() > def.capacity {
                report.error(
                    "over_capacity",
                    &cpath,
                    format!(
                        "holds {} but capacity is {}",
                        state.slots.len(),
                        def.capacity
                    ),
                );
            }
            for kid in &state.slots {
                listed_in.entry(*kid).or_default().push((id, cid.clone()));
                match world.objects.get(*kid) {
                    None => report.error(
                        "unknown_object_ref",
                        &cpath,
                        format!("lists {kid}, which does not exist"),
                    ),
                    Some(child) => {
                        match &child.location {
                            Location::InContainer(p) if p.owner == id && &p.container == cid => {}
                            _ => report.error(
                                "parent_mismatch",
                                &cpath,
                                format!("lists {kid}, but {kid} says it is elsewhere"),
                            ),
                        }
                        if let Some(ct) = content.get(&child.template) {
                            if !def.accepts.accepts(ct) {
                                report.error(
                                    "not_accepted",
                                    &cpath,
                                    format!("{kid} ({}) is not accepted here", child.template),
                                );
                            }
                        }
                    }
                }
            }
        }

        match &obj.location {
            Location::OnMap { map, tile } => match world.maps.get(*map) {
                None => report.error("bad_location", &path, format!("on unknown map {map}")),
                Some(m) if !m.in_bounds(*tile) => report.error(
                    "bad_location",
                    &path,
                    format!("tile {tile} is outside {map}"),
                ),
                Some(_) => {}
            },
            Location::InContainer(p) => match world.objects.get(p.owner) {
                None => report.error(
                    "unknown_object_ref",
                    &path,
                    format!("inside unknown object {}", p.owner),
                ),
                Some(owner) => match owner.containers.get(&p.container) {
                    None => report.error(
                        "unknown_container",
                        &path,
                        format!("{} has no container '{}'", p.owner, p.container),
                    ),
                    Some(c) if !c.slots.contains(&id) => {
                        report.error(
                            "parent_mismatch",
                            &path,
                            format!(
                                "says it is in {}.{}, which does not list it",
                                p.owner, p.container
                            ),
                        );
                    }
                    Some(_) => {}
                },
            },
        }

        // Cycle: the parent chain must reach a map within `len` steps.
        let mut current = id;
        let mut steps = 0usize;
        while let Some(Location::InContainer(p)) = world.objects.get(current).map(|o| &o.location) {
            current = p.owner;
            steps += 1;
            if steps > world.objects.len() {
                report.error("cycle", &path, "containment cycle");
                break;
            }
        }
    }

    for (kid, places) in &listed_in {
        if places.len() > 1 {
            let where_ = places
                .iter()
                .map(|(o, c)| format!("{o}.{c}"))
                .collect::<Vec<_>>()
                .join(", ");
            report.error(
                "duplicate_child",
                kid.to_string(),
                format!("is listed in several places: {where_}"),
            );
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::MapKind;
    use pg_content::{load_pack, ComponentRegistry, Limits, MemoryPack};
    use proptest::prelude::*;

    fn content() -> ContentSet {
        let pack = MemoryPack::new()
            .with("pack.json", r#"{"id":"base","name":"Base","version":"0.1.0"}"#)
            .with(
                "data/templates/t.json",
                r#"[
                {"id":"base.object","schema":1},
                {"id":"base.item","schema":1,"extends":"base.object","tags":["item"]},
                {"id":"base.furniture","schema":1,"extends":"base.item","tags":["furniture"]},
                {"id":"item.coin","schema":1,"extends":"base.item"},
                {"id":"item.gem","schema":1,"extends":"base.item","tags":["precious"]},
                {"id":"furniture.drawer","schema":1,"extends":"base.furniture",
                 "containers":[{"id":"contents","capacity":2,"accepts":{"tags":["item"],"exclude_tags":["furniture"]}}]},
                {"id":"furniture.safe","schema":1,"extends":"base.furniture",
                 "containers":[{"id":"vault","capacity":3,"accepts":{"tags":["precious"]}}]},
                {"id":"furniture.cabinet","schema":1,"extends":"base.furniture",
                 "containers":[{"id":"shelf","capacity":4,"accepts":{"templates":["furniture.drawer"]}}]},
                {"id":"furniture.crate","schema":1,"extends":"base.furniture",
                 "containers":[{"id":"inner","capacity":2,"accepts":{"templates":["furniture.crate"]}}]}
            ]"#,
            );
        let loaded = load_pack(&pack, &Limits::default()).unwrap();
        ContentSet::build(vec![loaded], ComponentRegistry::builtin()).unwrap()
    }

    fn tid(s: &str) -> TemplateId {
        s.parse().unwrap()
    }

    fn world() -> (WorldState, EntityId) {
        let mut w = WorldState::new("t", "s");
        let m = w.create_map(MapKind::Overworld, 10, 10).unwrap();
        (w, m)
    }

    fn on(map: EntityId, x: i32, y: i32) -> Location {
        Location::OnMap {
            map,
            tile: Tile::new(x, y),
        }
    }

    fn inside(owner: EntityId, container: &str) -> Location {
        Location::InContainer(Parent {
            owner,
            container: container.into(),
        })
    }

    #[test]
    fn spawning_creates_container_state_from_the_template() {
        let (mut w, m) = world();
        let c = content();
        let drawer = spawn_object(&mut w, &c, &tid("furniture.drawer"), on(m, 1, 1)).unwrap();
        assert!(w
            .objects
            .get(drawer)
            .unwrap()
            .containers
            .contains_key("contents"));
        let coin = spawn_object(&mut w, &c, &tid("item.coin"), inside(drawer, "contents")).unwrap();
        assert_eq!(
            w.objects.get(drawer).unwrap().containers["contents"].slots,
            vec![coin]
        );
        assert!(validate_containment(&w, &c).is_ok());
    }

    #[test]
    fn capacity_is_enforced_and_failures_do_not_burn_ids() {
        let (mut w, m) = world();
        let c = content();
        let drawer = spawn_object(&mut w, &c, &tid("furniture.drawer"), on(m, 1, 1)).unwrap();
        spawn_object(&mut w, &c, &tid("item.coin"), inside(drawer, "contents")).unwrap();
        spawn_object(&mut w, &c, &tid("item.coin"), inside(drawer, "contents")).unwrap();
        let before = (w.id_counters.clone(), w.objects.len());
        assert!(matches!(
            spawn_object(&mut w, &c, &tid("item.coin"), inside(drawer, "contents")),
            Err(ContainmentError::Full { .. })
        ));
        assert_eq!(
            (w.id_counters.clone(), w.objects.len()),
            before,
            "a refused spawn changes nothing"
        );
    }

    #[test]
    fn accept_rules_use_tags_exclusions_and_template_lists() {
        let (mut w, m) = world();
        let c = content();
        let drawer = spawn_object(&mut w, &c, &tid("furniture.drawer"), on(m, 1, 1)).unwrap();
        let safe = spawn_object(&mut w, &c, &tid("furniture.safe"), on(m, 2, 1)).unwrap();
        let cabinet = spawn_object(&mut w, &c, &tid("furniture.cabinet"), on(m, 3, 1)).unwrap();
        let coin = spawn_object(&mut w, &c, &tid("item.coin"), on(m, 4, 1)).unwrap();
        let gem = spawn_object(&mut w, &c, &tid("item.gem"), on(m, 5, 1)).unwrap();
        // The drawer takes items but excludes furniture (a safe is furniture and an item).
        assert!(move_into(&mut w, &c, coin, drawer, "contents").is_ok());
        assert!(matches!(
            move_into(&mut w, &c, safe, drawer, "contents"),
            Err(ContainmentError::NotAccepted { .. })
        ));
        // The safe takes only precious things.
        assert!(matches!(
            move_into(&mut w, &c, coin, safe, "vault"),
            Err(ContainmentError::NotAccepted { .. })
        ));
        assert!(move_into(&mut w, &c, gem, safe, "vault").is_ok());
        // The cabinet takes only drawers, by template id.
        assert!(move_into(&mut w, &c, drawer, cabinet, "shelf").is_ok());
        assert!(matches!(
            move_into(&mut w, &c, safe, cabinet, "shelf"),
            Err(ContainmentError::NotAccepted { .. })
        ));
        assert!(validate_containment(&w, &c).is_ok());
    }

    #[test]
    fn cycles_are_refused() {
        let (mut w, m) = world();
        let c = content();
        let outer = spawn_object(&mut w, &c, &tid("furniture.crate"), on(m, 1, 1)).unwrap();
        let middle =
            spawn_object(&mut w, &c, &tid("furniture.crate"), inside(outer, "inner")).unwrap();
        let inner =
            spawn_object(&mut w, &c, &tid("furniture.crate"), inside(middle, "inner")).unwrap();
        // Into itself, into its own child, and into its own grandchild.
        assert!(matches!(
            move_into(&mut w, &c, outer, outer, "inner"),
            Err(ContainmentError::Cycle { .. })
        ));
        assert!(matches!(
            move_into(&mut w, &c, outer, middle, "inner"),
            Err(ContainmentError::Cycle { .. })
        ));
        assert!(matches!(
            move_into(&mut w, &c, outer, inner, "inner"),
            Err(ContainmentError::Cycle { .. })
        ));
        assert!(matches!(
            move_into(&mut w, &c, middle, inner, "inner"),
            Err(ContainmentError::Cycle { .. })
        ));
        assert!(
            validate_containment(&w, &c).is_ok(),
            "refused moves changed nothing"
        );
    }

    #[test]
    fn moving_between_containers_and_back_to_the_map() {
        let (mut w, m) = world();
        let c = content();
        let d1 = spawn_object(&mut w, &c, &tid("furniture.drawer"), on(m, 1, 1)).unwrap();
        let d2 = spawn_object(&mut w, &c, &tid("furniture.drawer"), on(m, 2, 1)).unwrap();
        let coin = spawn_object(&mut w, &c, &tid("item.coin"), inside(d1, "contents")).unwrap();
        move_into(&mut w, &c, coin, d2, "contents").unwrap();
        assert!(w.objects.get(d1).unwrap().containers["contents"]
            .slots
            .is_empty());
        assert_eq!(
            w.objects.get(d2).unwrap().containers["contents"].slots,
            vec![coin]
        );
        move_to_tile(&mut w, &c, coin, m, Tile::new(5, 5)).unwrap();
        assert!(w.objects.get(d2).unwrap().containers["contents"]
            .slots
            .is_empty());
        assert_eq!(w.objects.get(coin).unwrap().location, on(m, 5, 5));
        assert!(matches!(
            move_to_tile(&mut w, &c, coin, m, Tile::new(50, 5)),
            Err(ContainmentError::OutOfBounds(_))
        ));
        assert!(validate_containment(&w, &c).is_ok());
    }

    #[test]
    fn a_failed_move_leaves_the_object_where_it_was() {
        let (mut w, m) = world();
        let c = content();
        let d1 = spawn_object(&mut w, &c, &tid("furniture.drawer"), on(m, 1, 1)).unwrap();
        let d2 = spawn_object(&mut w, &c, &tid("furniture.drawer"), on(m, 2, 1)).unwrap();
        let coin = spawn_object(&mut w, &c, &tid("item.coin"), inside(d1, "contents")).unwrap();
        spawn_object(&mut w, &c, &tid("item.coin"), inside(d2, "contents")).unwrap();
        spawn_object(&mut w, &c, &tid("item.coin"), inside(d2, "contents")).unwrap(); // d2 is full
        let before = w.clone();
        assert!(matches!(
            move_into(&mut w, &c, coin, d2, "contents"),
            Err(ContainmentError::Full { .. })
        ));
        assert_eq!(w, before);
        // Moving within the same full container is fine (it frees its own slot first).
        assert!(move_into(&mut w, &c, coin, d1, "contents").is_ok());
    }

    #[test]
    fn delete_policies() {
        let (mut w, m) = world();
        let c = content();
        let build = |w: &mut WorldState| {
            let cab = spawn_object(w, &c, &tid("furniture.cabinet"), on(m, 1, 1)).unwrap();
            let dr = spawn_object(w, &c, &tid("furniture.drawer"), inside(cab, "shelf")).unwrap();
            let coin = spawn_object(w, &c, &tid("item.coin"), inside(dr, "contents")).unwrap();
            (cab, dr, coin)
        };
        // Forbid: refused while it holds something; allowed when empty.
        let (cab, dr, coin) = build(&mut w);
        assert_eq!(
            delete_object(&mut w, &c, dr, DeletePolicy::Forbid),
            Err(ContainmentError::HasChildren(dr))
        );
        assert_eq!(
            delete_object(&mut w, &c, coin, DeletePolicy::Forbid).unwrap(),
            vec![coin]
        );
        assert_eq!(
            delete_object(&mut w, &c, dr, DeletePolicy::Forbid).unwrap(),
            vec![dr]
        );
        assert!(
            w.objects.get(cab).unwrap().containers["shelf"]
                .slots
                .is_empty(),
            "removed from its parent's list"
        );

        // Cascade removes the whole subtree.
        let (cab2, dr2, coin2) = build(&mut w);
        let mut gone = delete_object(&mut w, &c, cab2, DeletePolicy::Cascade).unwrap();
        gone.sort();
        let mut want = vec![cab2, dr2, coin2];
        want.sort();
        assert_eq!(gone, want);
        assert!(!w.objects.contains(dr2) && !w.objects.contains(coin2));

        // Evict puts the children where the deleted object was. On the map: they land on its tile.
        let drawer = spawn_object(&mut w, &c, &tid("furniture.drawer"), on(m, 7, 7)).unwrap();
        let coin_a =
            spawn_object(&mut w, &c, &tid("item.coin"), inside(drawer, "contents")).unwrap();
        let coin_b =
            spawn_object(&mut w, &c, &tid("item.coin"), inside(drawer, "contents")).unwrap();
        assert_eq!(
            delete_object(&mut w, &c, drawer, DeletePolicy::Evict).unwrap(),
            vec![drawer]
        );
        assert_eq!(w.objects.get(coin_a).unwrap().location, on(m, 7, 7));
        assert_eq!(w.objects.get(coin_b).unwrap().location, on(m, 7, 7));

        // Evict fails (changing nothing) when the parent container will not take the children:
        // a shelf accepts only drawers, so evicting a drawer's coins into it is refused.
        let (_cab3, dr3, _coin3) = build(&mut w);
        let before = w.clone();
        assert!(matches!(
            delete_object(&mut w, &c, dr3, DeletePolicy::Evict),
            Err(ContainmentError::NotAccepted { .. })
        ));
        assert_eq!(w, before);
        assert!(validate_containment(&w, &c).is_ok());
    }

    #[test]
    fn validation_reports_every_kind_of_corruption() {
        let (mut w, m) = world();
        let c = content();
        let drawer = spawn_object(&mut w, &c, &tid("furniture.drawer"), on(m, 1, 1)).unwrap();
        let coin = spawn_object(&mut w, &c, &tid("item.coin"), inside(drawer, "contents")).unwrap();
        assert!(validate_containment(&w, &c).is_ok());

        // Parent says one thing, child another.
        let mut bad = w.clone();
        bad.objects.get_mut(coin).unwrap().location = on(m, 3, 3);
        assert!(validate_containment(&bad, &c).has_code("parent_mismatch"));

        // Listed twice.
        let mut bad = w.clone();
        bad.objects
            .get_mut(drawer)
            .unwrap()
            .containers
            .get_mut("contents")
            .unwrap()
            .slots
            .push(coin);
        let r = validate_containment(&bad, &c);
        assert!(r.has_code("duplicate_child"), "{r}");

        // Over capacity. (Create the extra objects first: spawning into an already-corrupt world would
        // trip the debug assertion, which is the point of that assertion.)
        let mut bad = w.clone();
        let extras: Vec<EntityId> = (0..3)
            .map(|_| spawn_object(&mut bad, &c, &tid("item.coin"), on(m, 6, 6)).unwrap())
            .collect();
        for extra in extras {
            bad.objects
                .get_mut(drawer)
                .unwrap()
                .containers
                .get_mut("contents")
                .unwrap()
                .slots
                .push(extra);
        }
        assert!(validate_containment(&bad, &c).has_code("over_capacity"));

        // A cycle.
        let mut bad = w.clone();
        bad.objects.get_mut(drawer).unwrap().location = inside(coin, "contents");
        assert!(validate_containment(&bad, &c).has_code("cycle"));

        // Dangling references and bad locations.
        let mut bad = w.clone();
        bad.objects.remove(coin);
        assert!(validate_containment(&bad, &c).has_code("unknown_object_ref"));
        let mut bad = w.clone();
        bad.objects.get_mut(drawer).unwrap().location = on(EntityId::new(Kind::Map, 9), 0, 0);
        assert!(validate_containment(&bad, &c).has_code("bad_location"));
        let mut bad = w.clone();
        bad.objects.get_mut(drawer).unwrap().containers.clear();
        assert!(validate_containment(&bad, &c).has_code("missing_container_state"));
    }

    #[derive(Clone, Debug)]
    enum Op {
        SpawnMap(usize, i32, i32),
        SpawnIn(usize, usize),
        MoveInto(usize, usize),
        ToTile(usize, i32, i32),
        Delete(usize, u8),
    }

    fn op_strategy() -> impl Strategy<Value = Op> {
        prop_oneof![
            (0usize..6, 0i32..10, 0i32..10).prop_map(|(t, x, y)| Op::SpawnMap(t, x, y)),
            (0usize..6, 0usize..40).prop_map(|(t, o)| Op::SpawnIn(t, o)),
            (0usize..40, 0usize..40).prop_map(|(a, b)| Op::MoveInto(a, b)),
            (0usize..40, 0i32..10, 0i32..10).prop_map(|(o, x, y)| Op::ToTile(o, x, y)),
            (0usize..40, 0u8..3).prop_map(|(o, p)| Op::Delete(o, p)),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(48))]

        #[test]
        fn invariants_hold_after_any_edit_sequence(ops in proptest::collection::vec(op_strategy(), 0..40)) {
            let templates = ["item.coin", "item.gem", "furniture.drawer", "furniture.safe", "furniture.cabinet", "furniture.crate"];
            let containers = ["contents", "vault", "shelf", "inner"];
            let c = content();
            let (mut w, m) = world();
            for op in ops {
                let ids: Vec<EntityId> = w.objects.ids().collect();
                let pick = |i: usize| ids.get(i % ids.len().max(1)).copied();
                // Errors are fine (that is the point); only invariant violations are bugs.
                match op {
                    Op::SpawnMap(t, x, y) => { let _ = spawn_object(&mut w, &c, &tid(templates[t % 6]), on(m, x, y)); }
                    Op::SpawnIn(t, o) => {
                        if let Some(owner) = pick(o) {
                            for cn in containers { let _ = spawn_object(&mut w, &c, &tid(templates[t % 6]), inside(owner, cn)); }
                        }
                    }
                    Op::MoveInto(a, b) => {
                        if let (Some(child), Some(owner)) = (pick(a), pick(b)) {
                            for cn in containers { let _ = move_into(&mut w, &c, child, owner, cn); }
                        }
                    }
                    Op::ToTile(o, x, y) => { if let Some(id) = pick(o) { let _ = move_to_tile(&mut w, &c, id, m, Tile::new(x, y)); } }
                    Op::Delete(o, p) => {
                        if let Some(id) = pick(o) {
                            let policy = [DeletePolicy::Cascade, DeletePolicy::Evict, DeletePolicy::Forbid][usize::from(p) % 3];
                            let _ = delete_object(&mut w, &c, id, policy);
                        }
                    }
                }
                let report = validate_containment(&w, &c);
                prop_assert!(report.is_ok(), "violated after {:?}:\n{}", op, report);
            }
        }
    }
}
