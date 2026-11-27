//! Merging two versions of a record (wiki: Per-Node-Sync §3.3).
//!
//! Each field keeps the write with the later clock. The merge is commutative, associative and
//! idempotent, so however records arrive (in any order, more than once) every device ends
//! with the same one.

use serde::Serialize;
use serde_json::Map;

use crate::wire::{NodeRecord, PrefsRecord, Reg};

/// The later of two writes of a field. Equal clocks mean the same write, so the values should
/// match; if they don't (bad data), the larger one, as JSON text, wins everywhere alike.
fn later<T: Clone + Serialize>(a: &Option<Reg<T>>, b: &Option<Reg<T>>) -> Option<Reg<T>> {
    match (a, b) {
        (Some(x), Some(y)) => Some(if (x.at, json(&x.v)) >= (y.at, json(&y.v)) { x.clone() } else { y.clone() }),
        (x, y) => x.clone().or_else(|| y.clone()),
    }
}

fn json<T: Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

/// Two versions of one record merged, field by field.
/// Merge two versions of the preferences: each key takes its later write.
pub fn merge_prefs(a: &PrefsRecord, b: &PrefsRecord) -> PrefsRecord {
    let mut values = a.values.clone();
    for (key, theirs) in &b.values {
        let mine = values.get(key).cloned();
        if let Some(later) = later(&mine, &Some(theirs.clone())) {
            values.insert(key.clone(), later);
        }
    }
    PrefsRecord { values }
}

pub fn merge(a: &NodeRecord, b: &NodeRecord) -> NodeRecord {
    debug_assert_eq!(a.id, b.id);
    let mut extra = Map::new();
    for (key, value) in a.extra.iter().chain(b.extra.iter()) {
        // Fields from a newer version: keep both sides', the larger where they differ.
        let keep = extra.get(key).is_none_or(|kept| json(kept) < json(value));
        if keep {
            extra.insert(key.clone(), value.clone());
        }
    }
    NodeRecord {
        id: a.id,
        created: match (a.created, b.created) {
            (Some(x), Some(y)) => Some(x.min(y)),
            (x, y) => x.or(y),
        },
        position: later(&a.position, &b.position),
        text: later(&a.text, &b.text),
        note: later(&a.note, &b.note),
        completed: later(&a.completed, &b.completed),
        kind: later(&a.kind, &b.kind),
        mirror: later(&a.mirror, &b.mirror),
        image: later(&a.image, &b.image),
        star: later(&a.star, &b.star),
        deleted: later(&a.deleted, &b.deleted),
        removed: a.removed.max(b.removed),
        extra,
    }
}
