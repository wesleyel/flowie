//! Devices with real trees syncing through the memory backend: whatever they do, and however
//! their pushes and pulls interleave, once everything is exchanged they hold the same records
//! and show the same outline.

use std::collections::BTreeMap;

use flowie_core::tree::{NodeId, Tree, ROOT};
use flowie_sync::{ChangeBatch, Engine, MemoryServer, MemoryStore, NodeRecord, RecordStore, SaveResult};
use proptest::prelude::*;

struct Device {
    store: MemoryStore,
    engine: Engine,
}

impl Device {
    fn new(actor: u64) -> Self {
        let mut tree = Tree::new();
        tree.set_actor(actor);
        Device { store: MemoryStore::new(tree), engine: Engine::new() }
    }

    fn push(&mut self, server: &mut MemoryServer, limit: usize, fail_every: Option<usize>) -> usize {
        let saves = self.engine.next_saves(&self.store, limit);
        let n = saves.len();
        let mut results = server.modify(saves);
        if let Some(k) = fail_every {
            for (i, r) in results.iter_mut().enumerate() {
                if i % k == 0 {
                    if let SaveResult::Saved { id, .. } = r {
                        // Saved, but the answer was lost: it is sent again, from an old version.
                        *r = SaveResult::Failed { id: id.clone(), error: "lost".into() };
                    }
                }
            }
        }
        let conflicts = results.iter().filter(|r| matches!(r, SaveResult::Conflict { .. })).count();
        self.engine.on_saved(&mut self.store, results);
        n.max(conflicts)
    }

    fn pull(&mut self, server: &MemoryServer, limit: usize) -> ChangeBatch {
        let batch = server.changes(self.store.cursor().as_deref(), limit);
        let more = batch.clone();
        self.engine.on_fetched(&mut self.store, batch);
        more
    }

    fn visible(&self) -> Vec<NodeId> {
        self.store.tree.descendants(ROOT)
    }

    /// Every record and tombstone this device holds.
    fn records(&self) -> BTreeMap<NodeId, NodeRecord> {
        let tree = &self.store.tree;
        let mut out: BTreeMap<NodeId, NodeRecord> =
            tree.records().filter(|n| n.id != ROOT).map(|n| (n.id, NodeRecord::from_node(n))).collect();
        for id in self.known_ids() {
            if let (false, Some(at)) = (out.contains_key(&id), tree.removed_at(id)) {
                out.insert(id, NodeRecord::tombstone(id, at));
            }
        }
        out
    }

    fn known_ids(&self) -> Vec<NodeId> {
        (1..=200u128).map(NodeId).collect::<Vec<_>>().into_iter().chain(self.store.tree.records().map(|n| n.id)).collect()
    }

    /// The outline as shown: each visible node with its parent and its place.
    fn outline(&self) -> Vec<(NodeId, NodeId)> {
        let tree = &self.store.tree;
        tree.descendants(ROOT).into_iter().map(|id| (id, tree.parent(id))).collect()
    }
}

/// One random step, chosen and parameterized by `seed`.
fn step(devices: &mut [Device], server: &mut MemoryServer, seed: u64, next_id: &mut u128) {
    let pick = |n: usize, k: u64| (k % n.max(1) as u64) as usize;
    let d = pick(devices.len(), seed);
    let r = seed >> 8;
    let dev = &mut devices[d];
    let visible = dev.visible();
    let any = |k: u64| if visible.is_empty() { ROOT } else { visible[pick(visible.len(), k)] };
    match r % 12 {
        0..=2 => {
            let parent = if r.is_multiple_of(3) { ROOT } else { any(r >> 4) };
            let at = pick(dev.store.tree.children(parent).len() + 1, r >> 9);
            // Ids the test can name, so tombstones can be compared across devices.
            let id = NodeId(*next_id);
            *next_id += 1;
            let mut node = flowie_core::tree::Node::new(id, parent, dev.store.tree.rank_at(parent, at));
            node.text = format!("n{}", id.0);
            dev.store.tree.put(node);
        }
        3 | 4 => {
            if !visible.is_empty() {
                let id = any(r >> 4);
                dev.store.tree.set_text(id, &format!("t{}", r % 97));
            }
        }
        5 | 6 => {
            if !visible.is_empty() {
                let (id, to) = (any(r >> 4), if r.is_multiple_of(5) { ROOT } else { any(r >> 12) });
                dev.store.tree.move_to(id, to, 0);
            }
        }
        7 => {
            if !visible.is_empty() {
                dev.store.tree.trash(any(r >> 4));
            }
        }
        8 => {
            let trashed = dev.store.tree.trashed();
            if !trashed.is_empty() {
                dev.store.tree.untrash(trashed[pick(trashed.len(), r >> 4)]);
            }
        }
        9 => {
            // A removal for good of a leaf (merging rows does this).
            let leaves: Vec<NodeId> = visible.iter().copied().filter(|id| !dev.store.tree.has_children(*id)).collect();
            if !leaves.is_empty() {
                dev.store.tree.remove(leaves[pick(leaves.len(), r >> 4)]);
            }
        }
        10 => {
            dev.push(server, 1 + pick(4, r >> 4), r.is_multiple_of(3).then_some(2));
        }
        _ => {
            dev.pull(server, 1 + pick(4, r >> 4));
        }
    }
    dev.store.collect();
    let tree = &dev.store.tree;
    let mut fresh = Tree::from_nodes(tree.records().cloned());
    fresh.set_purged(tree.purged());
    let fresh: Vec<(NodeId, NodeId)> = fresh.descendants(ROOT).into_iter().map(|id| (id, fresh.parent(id))).collect();
    assert_eq!(dev.outline(), fresh, "after step kind {} on device {d}", r % 12);
}

fn settle(devices: &mut [Device], server: &mut MemoryServer) {
    for _ in 0..40 {
        let mut busy = false;
        for dev in devices.iter_mut() {
            while dev.push(server, 50, None) > 0 {
                busy = true;
            }
        }
        for dev in devices.iter_mut() {
            loop {
                let batch = dev.pull(server, 50);
                busy |= !batch.records.is_empty();
                if !batch.more {
                    break;
                }
            }
            dev.store.collect();
        }
        if !busy && devices.iter().all(|d| !d.store.has_unsent()) {
            return;
        }
    }
    panic!("sync didn't settle");
}

fn run(steps: &[u64]) {
    let mut devices: Vec<Device> = (1..=3).map(Device::new).collect();
    let mut server = MemoryServer::new();
    let mut next_id = 1;
    for &seed in steps {
        step(&mut devices, &mut server, seed, &mut next_id);
    }
    settle(&mut devices, &mut server);
    for (i, dev) in devices.iter().enumerate() {
        let tree = &dev.store.tree;
        let mut fresh = Tree::from_nodes(tree.records().cloned());
        fresh.set_purged(tree.purged());
        let fresh: Vec<(NodeId, NodeId)> = fresh.descendants(ROOT).into_iter().map(|id| (id, fresh.parent(id))).collect();
        assert_eq!(dev.outline(), fresh, "device {i} shows what its records give");
    }
    let first = devices[0].records();
    let outline = devices[0].outline();
    for (i, dev) in devices.iter().enumerate().skip(1) {
        let mine = dev.records();
        let differ: Vec<String> = first
            .keys()
            .chain(mine.keys())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .filter(|id| first.get(id) != mine.get(id))
            .map(|id| format!("{id}:\n  device 0: {:?}\n  device {i}: {:?}", first.get(id), mine.get(id)))
            .collect();
        assert!(differ.is_empty(), "every device holds the same records; these differ:\n{}", differ.join("\n"));
        assert_eq!(dev.outline(), outline, "and shows the same outline");
    }
    // What each device shows is what its records give, derived from scratch.
    for (i, dev) in devices.iter().enumerate() {
        let tree = &dev.store.tree;
        let mut fresh = Tree::from_nodes(tree.records().cloned());
        fresh.set_purged(tree.purged());
        let fresh: Vec<(NodeId, NodeId)> = fresh.descendants(ROOT).into_iter().map(|id| (id, fresh.parent(id))).collect();
        assert_eq!(dev.outline(), fresh, "device {i} shows what its records give");
    }
    // The outline is a tree: every shown node's parent chain reaches the root.
    let tree = &devices[0].store.tree;
    for (id, _) in &outline {
        let mut n = *id;
        for _ in 0..=outline.len() {
            if n == ROOT {
                break;
            }
            n = tree.parent(n);
        }
        assert_eq!(n, ROOT, "{id} is shown in a cycle");
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn devices_converge(steps in proptest::collection::vec(any::<u64>(), 1..120)) {
        run(&steps);
    }
}

#[test]
fn a_conflicting_save_merges_and_goes_out_again() {
    let (mut a, mut b) = (Device::new(1), Device::new(2));
    let mut server = MemoryServer::new();
    let id = a.store.tree.create_at(ROOT, 0, "first");
    a.store.collect();
    settle(&mut [], &mut server);
    a.push(&mut server, 10, None);
    b.pull(&server, 10);
    // Both edit the node; b saves first, so a's save is a conflict.
    b.store.tree.set_note(id, "b's note");
    b.store.collect();
    a.store.tree.set_text(id, "a's text");
    a.store.collect();
    b.push(&mut server, 10, None);
    a.push(&mut server, 10, None);
    assert!(a.store.has_unsent(), "a merged b's version and sends again");
    let mut devices = [a, b];
    settle(&mut devices, &mut server);
    for dev in &devices {
        let node = dev.store.tree.node(id);
        assert_eq!((node.text.as_str(), node.note.as_str()), ("a's text", "b's note"), "both edits survive");
    }
}
