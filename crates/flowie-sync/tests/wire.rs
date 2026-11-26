//! The wire format against fixed data, and the merge's laws.

use flowie_core::hlc::Hlc;
use flowie_core::tree::{BlockKind, NodeId, ROOT};
use flowie_sync::{merge, Envelope, NodeRecord, Position, Reg};
use proptest::prelude::*;
use serde_json::Value;

const GOLDEN: &str = include_str!("../testdata/node-v1.json");

fn hlc(wall: u64, counter: u16, actor: u64) -> Hlc {
    Hlc { wall, counter, actor }
}

#[test]
fn a_record_from_the_golden_file_reads_and_writes_back_unchanged() {
    let record: NodeRecord = serde_json::from_str(GOLDEN).unwrap();
    let written: Value = serde_json::to_value(&record).unwrap();
    let original: Value = serde_json::from_str(GOLDEN).unwrap();
    assert_eq!(written, original, "every field, a newer version's included, goes back as it came");
    assert_eq!(record.text.as_ref().unwrap().at, hlc(0x0193a0000100, 0, 7));
    assert!(record.extra.contains_key("color"));

    let node = record.to_node(None).unwrap();
    assert_eq!(node.id, NodeId(10));
    assert_eq!(node.text, "Ship **1.1**");
    assert_eq!(node.kind, BlockKind::Todo);
    assert_eq!((node.parent, node.sync.moved_from), (ROOT, NodeId(11)));
    assert_eq!(node.star, "a0");
    // Back to a record: the same, a newer version's field included.
    assert_eq!(serde_json::to_value(NodeRecord::from_node(&node)).unwrap(), original);
    // And in an envelope.
    let envelope = Envelope::node(&record, Some("7".into()));
    let envelope: Envelope = serde_json::from_str(&serde_json::to_string(&envelope).unwrap()).unwrap();
    assert_eq!(envelope.node_record(), Some(record));
}

#[test]
fn an_envelope_of_an_unknown_kind_is_kept_but_not_read() {
    let envelope: Envelope = serde_json::from_str(r#"{"v":1,"kind":"later","id":"x","payload":{}}"#).unwrap();
    assert_eq!(envelope.kind, flowie_sync::Kind::Unknown);
    assert_eq!(envelope.node_record(), None);
}

#[test]
fn a_write_after_a_removal_keeps_the_node() {
    let mut record: NodeRecord = serde_json::from_str(GOLDEN).unwrap();
    let latest = record.latest_write().unwrap();
    record.removed = Some(hlc(latest.wall + 1, 0, 1));
    assert!(!record.exists() && record.to_node(None).is_none());
    let mut text = record.text.clone().unwrap();
    text.at = hlc(latest.wall + 2, 0, 2);
    record.text = Some(text);
    assert!(record.exists(), "written after it was removed");
    assert!(!NodeRecord::tombstone(NodeId(1), hlc(1, 0, 1)).exists());
}

fn clock() -> impl Strategy<Value = Hlc> {
    (0u64..4, 0u16..2, 0u64..3).prop_map(|(w, c, a)| hlc(w, c, a))
}

fn reg<T: std::fmt::Debug + Clone>(values: impl Strategy<Value = T>) -> impl Strategy<Value = Option<Reg<T>>> {
    proptest::option::of((values, clock()).prop_map(|(v, at)| Reg { v, at }))
}

fn record() -> impl Strategy<Value = NodeRecord> {
    let ids = prop_oneof![Just(ROOT), Just(NodeId(1)), Just(NodeId(2))];
    (
        proptest::option::of(clock()),
        reg((ids.clone(), "[ab]", ids.clone()).prop_map(|(parent, rank, from)| Position { parent, rank, from })),
        reg("[xy]{0,2}"),
        reg(any::<bool>()),
        reg(proptest::option::of(0i64..3)),
        proptest::option::of(clock()),
        proptest::collection::btree_map("[pq]", 0u8..3, 0..2),
    )
        .prop_map(|(created, position, text, completed, deleted, removed, extra)| NodeRecord {
            created,
            position,
            text,
            completed,
            deleted,
            removed,
            extra: extra.into_iter().map(|(k, v)| (k, Value::from(v))).collect(),
            ..NodeRecord::empty(NodeId(9))
        })
}

proptest! {
    #[test]
    fn merging_is_commutative_associative_and_idempotent(a in record(), b in record(), c in record()) {
        prop_assert_eq!(merge(&a, &b), merge(&b, &a));
        prop_assert_eq!(merge(&merge(&a, &b), &c), merge(&a, &merge(&b, &c)));
        prop_assert_eq!(merge(&a, &a), a.clone());
        let ab = merge(&a, &b);
        prop_assert_eq!(merge(&ab, &b), ab);
    }
}
