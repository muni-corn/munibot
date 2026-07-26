use std::collections::BTreeMap;

/// The difference between two states of a `BTreeMap<K, V>`-backed
/// `Collection` field, e.g. `AutoDeleteConfig::timers` before and after a
/// patch.
///
/// A `ConfigurationProvider::write` for a provider with a collection field
/// needs to turn "here's the whole new map" into a minimal set of
/// database inserts, updates, and deletes; comparing `previous` and
/// `desired` key by key is what this does, so the provider doesn't have
/// to.
pub struct CollectionDelta<'a, K, V> {
    /// Keys present in `desired` but not `previous`.
    pub added: Vec<(&'a K, &'a V)>,
    /// Keys present in both, whose value differs.
    pub changed: Vec<(&'a K, &'a V)>,
    /// Keys present in `previous` but not `desired`.
    pub removed: Vec<&'a K>,
}

/// Computes the difference between `previous` and `desired`.
///
/// Both are `BTreeMap`s, so `added`/`changed`/`removed` are each already in
/// key order.
pub fn collection_diff<'a, K: Ord, V: PartialEq>(
    previous: &'a BTreeMap<K, V>,
    desired: &'a BTreeMap<K, V>,
) -> CollectionDelta<'a, K, V> {
    let mut added = Vec::new();
    let mut changed = Vec::new();

    for (key, value) in desired {
        match previous.get(key) {
            None => added.push((key, value)),
            Some(previous_value) if previous_value != value => changed.push((key, value)),
            Some(_) => {}
        }
    }

    let removed = previous
        .keys()
        .filter(|key| !desired.contains_key(key))
        .collect();

    CollectionDelta {
        added,
        changed,
        removed,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::collection_diff;

    fn map(pairs: &[(u32, &str)]) -> BTreeMap<u32, String> {
        pairs.iter().map(|(k, v)| (*k, (*v).to_owned())).collect()
    }

    #[test]
    fn test_diffing_two_empty_maps_has_nothing() {
        let previous: BTreeMap<u32, String> = BTreeMap::new();
        let desired: BTreeMap<u32, String> = BTreeMap::new();
        let delta = collection_diff(&previous, &desired);
        assert!(delta.added.is_empty());
        assert!(delta.changed.is_empty());
        assert!(delta.removed.is_empty());
    }

    #[test]
    fn test_a_key_only_in_desired_is_added() {
        let previous = map(&[]);
        let desired = map(&[(1, "a")]);
        let delta = collection_diff(&previous, &desired);
        assert_eq!(delta.added, vec![(&1, &"a".to_owned())]);
        assert!(delta.changed.is_empty());
        assert!(delta.removed.is_empty());
    }

    #[test]
    fn test_a_key_only_in_previous_is_removed() {
        let previous = map(&[(1, "a")]);
        let desired = map(&[]);
        let delta = collection_diff(&previous, &desired);
        assert!(delta.added.is_empty());
        assert!(delta.changed.is_empty());
        assert_eq!(delta.removed, vec![&1]);
    }

    #[test]
    fn test_a_key_with_a_different_value_is_changed() {
        let previous = map(&[(1, "a")]);
        let desired = map(&[(1, "b")]);
        let delta = collection_diff(&previous, &desired);
        assert!(delta.added.is_empty());
        assert_eq!(delta.changed, vec![(&1, &"b".to_owned())]);
        assert!(delta.removed.is_empty());
    }

    #[test]
    fn test_a_key_with_the_same_value_is_neither_added_nor_changed_nor_removed() {
        let previous = map(&[(1, "a")]);
        let desired = map(&[(1, "a")]);
        let delta = collection_diff(&previous, &desired);
        assert!(delta.added.is_empty());
        assert!(delta.changed.is_empty());
        assert!(delta.removed.is_empty());
    }

    #[test]
    fn test_a_mixed_diff_with_all_three_kinds_at_once() {
        let previous = map(&[(1, "unchanged"), (2, "old"), (3, "gone")]);
        let desired = map(&[(1, "unchanged"), (2, "new"), (4, "fresh")]);
        let delta = collection_diff(&previous, &desired);

        assert_eq!(delta.added, vec![(&4, &"fresh".to_owned())]);
        assert_eq!(delta.changed, vec![(&2, &"new".to_owned())]);
        assert_eq!(delta.removed, vec![&3]);
    }

    #[test]
    fn test_added_and_removed_are_in_key_order() {
        let previous = map(&[(5, "a"), (1, "b")]);
        let desired = map(&[(9, "c"), (2, "d")]);
        let delta = collection_diff(&previous, &desired);

        let added_keys: Vec<_> = delta.added.iter().map(|(k, _)| **k).collect();
        assert_eq!(added_keys, vec![2, 9]);

        let removed_keys: Vec<_> = delta.removed.iter().map(|k| **k).collect();
        assert_eq!(removed_keys, vec![1, 5]);
    }
}
