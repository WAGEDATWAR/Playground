//! `Table<T>`: id-keyed rows that always iterate in ascending id order (Blueprint §4.5, §5.2).
//!
//! `BTreeMap` is the default backing store because sorted iteration is a determinism requirement.
//! The API is deliberately small and panic-free; swapping the backing store for a dense sorted
//! `Vec` later must not change iteration order or hashes.

use crate::canon::{Canon, ToCanon};
use crate::id::EntityId;
use std::collections::BTreeMap;
use std::fmt;

/// An insert failed because the id is already present.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DuplicateId(pub EntityId);

impl fmt::Display for DuplicateId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "duplicate id {}", self.0)
    }
}

impl std::error::Error for DuplicateId {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table<T> {
    rows: BTreeMap<EntityId, T>,
}

impl<T> Default for Table<T> {
    fn default() -> Self {
        Table {
            rows: BTreeMap::new(),
        }
    }
}

impl<T> Table<T> {
    pub fn new() -> Table<T> {
        Table::default()
    }

    /// Inserts a new row. Existing rows are never overwritten; use [`Table::get_mut`] to change one.
    pub fn insert(&mut self, id: EntityId, row: T) -> Result<(), DuplicateId> {
        if self.rows.contains_key(&id) {
            return Err(DuplicateId(id));
        }
        self.rows.insert(id, row);
        Ok(())
    }

    pub fn get(&self, id: EntityId) -> Option<&T> {
        self.rows.get(&id)
    }

    pub fn get_mut(&mut self, id: EntityId) -> Option<&mut T> {
        self.rows.get_mut(&id)
    }

    pub fn contains(&self, id: EntityId) -> bool {
        self.rows.contains_key(&id)
    }

    pub fn remove(&mut self, id: EntityId) -> Option<T> {
        self.rows.remove(&id)
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Rows in ascending id order.
    pub fn iter(&self) -> impl Iterator<Item = (EntityId, &T)> {
        self.rows.iter().map(|(id, row)| (*id, row))
    }

    /// Mutable rows in ascending id order.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (EntityId, &mut T)> {
        self.rows.iter_mut().map(|(id, row)| (*id, row))
    }

    /// Ids in ascending order.
    pub fn ids(&self) -> impl Iterator<Item = EntityId> + '_ {
        self.rows.keys().copied()
    }
}

impl<T: ToCanon> ToCanon for Table<T> {
    /// A map from the id's text to the row. The text form is unique per id, so the canonical form
    /// does not depend on the table's (separate) iteration order.
    fn to_canon(&self) -> Canon {
        Canon::Map(
            self.iter()
                .map(|(id, row)| (id.to_string(), row.to_canon()))
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::Kind;
    use proptest::prelude::*;

    fn pawn(n: u32) -> EntityId {
        EntityId::new(Kind::Pawn, n)
    }

    #[test]
    fn tables_serialize_by_id_text() {
        let mut t = Table::new();
        t.insert(pawn(11), 7i32).unwrap();
        t.insert(pawn(2), 3i32).unwrap();
        assert_eq!(
            t.to_canon().to_canonical_string(),
            r#"{"pawn_2":3,"pawn_b":7}"#
        );
    }

    #[test]
    fn insert_get_remove() {
        let mut t = Table::new();
        assert!(t.is_empty());
        t.insert(pawn(1), "a").unwrap();
        assert_eq!(t.get(pawn(1)), Some(&"a"));
        assert!(t.contains(pawn(1)));
        assert_eq!(t.remove(pawn(1)), Some("a"));
        assert!(t.get(pawn(1)).is_none());
    }

    #[test]
    fn duplicate_insert_is_rejected_and_keeps_original() {
        let mut t = Table::new();
        t.insert(pawn(1), 10).unwrap();
        assert_eq!(t.insert(pawn(1), 20), Err(DuplicateId(pawn(1))));
        assert_eq!(t.get(pawn(1)), Some(&10));
    }

    #[test]
    fn iteration_is_ascending_regardless_of_insertion_order() {
        let mut t = Table::new();
        for n in [5, 1, 9, 3, 7] {
            t.insert(pawn(n), n).unwrap();
        }
        t.insert(EntityId::new(Kind::Map, 100), 0).unwrap();
        let ids: Vec<_> = t.ids().collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted);
        assert_eq!(ids.first(), Some(&EntityId::new(Kind::Map, 100)));
    }

    #[test]
    fn iter_mut_visits_in_order() {
        let mut t = Table::new();
        for n in [3, 1, 2] {
            t.insert(pawn(n), Vec::new()).unwrap();
        }
        let mut order = Vec::new();
        for (id, row) in t.iter_mut() {
            row.push(id.counter());
            order.push(id.counter());
        }
        assert_eq!(order, vec![1, 2, 3]);
    }

    proptest! {
        #[test]
        fn iteration_order_is_independent_of_insertion_order(mut ns in proptest::collection::vec(any::<u32>(), 0..40), seed in any::<u64>()) {
            ns.sort_unstable();
            ns.dedup();
            let mut a = Table::new();
            for &n in &ns { a.insert(pawn(n), n).unwrap(); }
            // Insert the same ids in a pseudo-shuffled order.
            let mut shuffled = ns.clone();
            let mut state = seed;
            for i in (1..shuffled.len()).rev() {
                state = crate::rng::mix64(state);
                shuffled.swap(i, (state % (i as u64 + 1)) as usize);
            }
            let mut b = Table::new();
            for &n in &shuffled { b.insert(pawn(n), n).unwrap(); }
            prop_assert_eq!(a.iter().map(|(i, v)| (i, *v)).collect::<Vec<_>>(),
                            b.iter().map(|(i, v)| (i, *v)).collect::<Vec<_>>());
        }
    }
}
