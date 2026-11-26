//! A model of per-node state sync, to check the merge and view rules before they go into
//! `Tree` and `flowie-sync` (docs/roadmap/per-node-sync.md §3.3–3.4, release-plan.md §4).
//!
//! It is separate from `Tree` on purpose: ids and ranks are integers, and a record has only the
//! fields the rules care about. Each replica holds records whose fields are last-writer-wins
//! registers stamped with a hybrid logical clock, plus a grow-only set of purged ids. Replicas
//! edit locally and swap random subsets of their records in random order, duplicates included;
//! at the end they sync fully. Then every replica must hold the same records and derive the
//! same outline, with no cycles, nothing left waiting for a parent, no record lost, and nothing
//! hidden that was created after the last deletion on its path.

use std::collections::{BTreeMap, BTreeSet};

use proptest::collection::vec;
use proptest::prelude::*;

type Id = u32;
const ROOT: Id = 0;
/// Where a record shows while its parent hasn't arrived yet.
const SYNCING: Id = u32::MAX;
/// How long (in model time) a deleted subtree stays before a replica may purge it.
const RETENTION: u64 = 60;

/// Hybrid logical clock, totally ordered by (wall, counter, actor).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Hlc {
    wall: u64,
    counter: u32,
    actor: u8,
}

/// A last-writer-wins register.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Reg<T> {
    val: T,
    at: Hlc,
}

impl<T: Clone> Reg<T> {
    fn new(val: T, at: Hlc) -> Self {
        Reg { val, at }
    }

    fn merge(&mut self, other: &Self) {
        if other.at > self.at {
            *self = other.clone();
        }
    }
}

/// The position is one register, so a move is atomic. `from` is the parent the node was moved
/// away from: a register keeps only its latest value, so the view needs `from` to undo a move
/// that closes a cycle.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Pos {
    parent: Id,
    rank: u32,
    from: Id,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Rec {
    /// Written once, by the replica that creates the node.
    created: Hlc,
    pos: Reg<Pos>,
    text: Reg<u32>,
    deleted: Reg<bool>,
}

impl Rec {
    fn merge(&mut self, other: &Rec) {
        self.pos.merge(&other.pos);
        self.text.merge(&other.text);
        self.deleted.merge(&other.deleted);
    }

    fn clocks(&self) -> [Hlc; 3] {
        [self.pos.at, self.text.at, self.deleted.at]
    }
}

struct Replica {
    actor: u8,
    /// How far this replica's wall clock is off.
    skew: i64,
    clock: Hlc,
    records: BTreeMap<Id, Rec>,
    purged: BTreeSet<Id>,
}

impl Replica {
    fn new(actor: u8, skew: i64) -> Self {
        Replica { actor, skew, clock: Hlc { actor, ..Hlc::default() }, records: BTreeMap::new(), purged: BTreeSet::new() }
    }

    fn wall(&self, t: u64) -> u64 {
        (t as i64 + self.skew).max(0) as u64
    }

    fn tick(&mut self, t: u64) -> Hlc {
        let wall = self.wall(t);
        self.clock = if wall > self.clock.wall {
            Hlc { wall, counter: 0, actor: self.actor }
        } else {
            Hlc { counter: self.clock.counter + 1, ..self.clock }
        };
        self.clock
    }

    /// Every clock seen moves ours forward, so a later local write sorts after it.
    fn observe(&mut self, at: Hlc) {
        if (at.wall, at.counter) > (self.clock.wall, self.clock.counter) {
            self.clock.wall = at.wall;
            self.clock.counter = at.counter;
        }
    }

    fn receive(&mut self, id: Id, rec: &Rec) {
        for at in rec.clocks() {
            self.observe(at);
        }
        if self.purged.contains(&id) {
            return;
        }
        match self.records.get_mut(&id) {
            Some(mine) => mine.merge(rec),
            None => {
                self.records.insert(id, rec.clone());
            }
        }
    }

    fn receive_purge(&mut self, id: Id) {
        self.purged.insert(id);
        self.records.remove(&id);
    }

    fn view(&self) -> View {
        View::derive(&self.records, &self.purged)
    }
}

/// The outline derived from the records. Never written back, so every replica holding the
/// same records derives the same view.
#[derive(Debug, PartialEq, Eq)]
struct View {
    parent: BTreeMap<Id, Id>,
    children: BTreeMap<Id, Vec<Id>>,
    /// Deleted subtree roots whose deletion stands.
    deleted: BTreeSet<Id>,
    reachable: BTreeSet<Id>,
    /// Nodes whose latest move the view undid to break a cycle.
    undone: BTreeMap<Id, u32>,
}

impl View {
    fn derive(records: &BTreeMap<Id, Rec>, purged: &BTreeSet<Id>) -> View {
        // Parents. A missing parent is still on its way, unless it was purged: then the record
        // shows at the top level rather than disappear with it.
        let mut parent: BTreeMap<Id, Id> = records
            .iter()
            .map(|(&id, r)| {
                let p = r.pos.val.parent;
                let p = if p == ROOT || records.contains_key(&p) {
                    p
                } else if purged.contains(&p) {
                    ROOT
                } else {
                    SYNCING
                };
                (id, p)
            })
            .collect();

        // Cycles. Concurrent moves can put A under B and B under A. The latest move in the
        // cycle is undone in the view: that node goes back where it was moved from, or to the
        // root when that parent is gone or it closes a cycle again.
        let mut undone: BTreeMap<Id, u32> = BTreeMap::new();
        while let Some(cycle) = find_cycle(&parent) {
            let n = *cycle.iter().max_by_key(|&&n| records[&n].pos.at).unwrap();
            let times = undone.entry(n).or_insert(0);
            *times += 1;
            let from = records[&n].pos.val.from;
            let back = *times == 1 && from != n && (from == ROOT || records.contains_key(&from));
            parent.insert(n, if back { from } else { ROOT });
        }

        let mut children: BTreeMap<Id, Vec<Id>> = BTreeMap::new();
        for (&id, &p) in &parent {
            children.entry(p).or_default().push(id);
        }
        for list in children.values_mut() {
            list.sort_by_key(|id| (records[id].pos.val.rank, *id));
        }

        // Deletion. A deleted subtree stays deleted unless something not itself deleted was
        // created under it after the deletion, by a device that hadn't seen it yet: then the
        // subtree comes back rather than hide the new content.
        let deleted: BTreeSet<Id> = records
            .iter()
            .filter(|(_, r)| r.deleted.val)
            .filter(|&(&id, r)| {
                !subtree(&children, id).iter().skip(1).any(|d| {
                    let dr = &records[d];
                    !dr.deleted.val && dr.created > r.deleted.at
                })
            })
            .map(|(&id, _)| id)
            .collect();

        // Reachable: the path to the root passes no standing deletion and nothing waiting.
        let mut reachable = BTreeSet::new();
        for &id in records.keys() {
            let mut n = id;
            let mut steps = 0;
            let shown = loop {
                if n == ROOT {
                    break true;
                }
                if n == SYNCING || deleted.contains(&n) {
                    break false;
                }
                n = parent[&n];
                steps += 1;
                assert!(steps <= records.len(), "the view has a cycle through {id}");
            };
            if shown {
                reachable.insert(id);
            }
        }
        View { parent, children, deleted, reachable, undone }
    }

    fn subtree(&self, id: Id) -> Vec<Id> {
        subtree(&self.children, id)
    }
}

/// `id` and everything below it in `children`.
fn subtree(children: &BTreeMap<Id, Vec<Id>>, id: Id) -> Vec<Id> {
    let mut out = vec![id];
    let mut i = 0;
    while i < out.len() {
        if let Some(list) = children.get(&out[i]) {
            out.extend(list);
        }
        i += 1;
    }
    out
}

fn find_cycle(parent: &BTreeMap<Id, Id>) -> Option<Vec<Id>> {
    let mut done = BTreeSet::new();
    for &start in parent.keys() {
        let mut path = Vec::new();
        let mut on_path = BTreeMap::new();
        let mut n = start;
        while n != ROOT && n != SYNCING && !done.contains(&n) {
            if let Some(&i) = on_path.get(&n) {
                return Some(path[i..].to_vec());
            }
            on_path.insert(n, path.len());
            path.push(n);
            n = parent[&n];
        }
        done.extend(path);
    }
    None
}

/// splitmix64, seeded per step so a history replays exactly.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    fn pick(&mut self, xs: &[Id]) -> Option<Id> {
        (!xs.is_empty()).then(|| xs[self.below(xs.len() as u64) as usize])
    }

    fn shuffle<T>(&mut self, xs: &mut [T]) {
        for i in (1..xs.len()).rev() {
            xs.swap(i, self.below(i as u64 + 1) as usize);
        }
    }
}

/// How often the random histories hit each rule, so a weak generator can't pass quietly.
#[derive(Default, Debug)]
struct Stats {
    cycles: usize,
    waiting: usize,
    revived: usize,
    restores: usize,
    purges: usize,
    purged_parents: usize,
}

struct Sim {
    stats: Stats,
    rng: Rng,
    /// True time; each replica reads it through its own skew.
    t: u64,
    replicas: Vec<Replica>,
    next_id: Id,
    created: BTreeSet<Id>,
}

impl Sim {
    fn new(seed: u64, skews: &[i64]) -> Self {
        let replicas = skews.iter().enumerate().map(|(i, &s)| Replica::new(i as u8 + 1, s)).collect();
        Sim { stats: Stats::default(), rng: Rng(seed), t: 100, replicas, next_id: 1, created: BTreeSet::new() }
    }

    fn advance(&mut self, dt: u64) {
        self.t += dt;
    }

    fn create(&mut self, r: usize, parent: Id) -> Id {
        let id = self.next_id;
        self.next_id += 1;
        self.created.insert(id);
        let rank = self.rng.below(1000) as u32;
        let rep = &mut self.replicas[r];
        let at = rep.tick(self.t);
        let pos = Reg::new(Pos { parent, rank, from: parent }, at);
        rep.records.insert(id, Rec { created: at, pos, text: Reg::new(0, at), deleted: Reg::new(false, at) });
        id
    }

    fn move_to(&mut self, r: usize, node: Id, parent: Id) {
        let from = self.replicas[r].view().parent[&node];
        let rank = self.rng.below(1000) as u32;
        let rep = &mut self.replicas[r];
        let at = rep.tick(self.t);
        rep.records.get_mut(&node).unwrap().pos = Reg::new(Pos { parent, rank, from }, at);
    }

    fn edit(&mut self, r: usize, node: Id) {
        let text = self.rng.below(1000) as u32;
        let rep = &mut self.replicas[r];
        let at = rep.tick(self.t);
        rep.records.get_mut(&node).unwrap().text = Reg::new(text, at);
    }

    fn set_deleted(&mut self, r: usize, node: Id, deleted: bool) {
        let rep = &mut self.replicas[r];
        let at = rep.tick(self.t);
        rep.records.get_mut(&node).unwrap().deleted = Reg::new(deleted, at);
    }

    /// Deletes a subtree for good: it leaves the records and joins the purged set.
    fn purge(&mut self, r: usize, root: Id) {
        for id in self.replicas[r].view().subtree(root) {
            self.replicas[r].receive_purge(id);
        }
    }

    /// Sends `a`'s records and purges to `b`: all of them, or (when `partial`) a random part,
    /// in random order, with some sent twice.
    fn send(&mut self, a: usize, b: usize, partial: bool) {
        let mut recs: Vec<(Id, Rec)> = self.replicas[a].records.iter().map(|(&id, r)| (id, r.clone())).collect();
        let mut purged: Vec<Id> = self.replicas[a].purged.iter().copied().collect();
        if partial {
            recs.retain(|_| self.rng.chance(60));
            purged.retain(|_| self.rng.chance(60));
            let again: Vec<(Id, Rec)> = recs.iter().filter(|_| self.rng.chance(10)).cloned().collect();
            recs.extend(again);
            self.rng.shuffle(&mut recs);
        }
        for id in purged {
            self.replicas[b].receive_purge(id);
        }
        for (id, rec) in &recs {
            self.replicas[b].receive(*id, rec);
        }
    }

    fn sync_all(&mut self) {
        let n = self.replicas.len();
        for _ in 0..2 {
            for a in 0..n {
                for b in 0..n {
                    if a != b {
                        self.send(a, b, false);
                    }
                }
            }
        }
    }

    /// One random local edit on replica `r`, on what it can see.
    fn random_edit(&mut self, r: usize) {
        let view = self.replicas[r].view();
        let visible: Vec<Id> = view.reachable.iter().copied().collect();
        let mut targets = visible.clone();
        targets.push(ROOT);
        match self.rng.below(10) {
            0..=3 => {
                let parent = self.rng.pick(&targets).unwrap();
                self.create(r, parent);
            }
            4 | 5 => {
                let Some(node) = self.rng.pick(&visible) else { return };
                let below: BTreeSet<Id> = view.subtree(node).into_iter().collect();
                let allowed: Vec<Id> = targets.iter().copied().filter(|t| !below.contains(t)).collect();
                let parent = self.rng.pick(&allowed).unwrap();
                self.move_to(r, node, parent);
            }
            6 => {
                if let Some(node) = self.rng.pick(&visible) {
                    self.edit(r, node);
                }
            }
            7 => {
                if let Some(node) = self.rng.pick(&visible) {
                    self.set_deleted(r, node, true);
                }
            }
            8 => {
                let restorable: Vec<Id> = view
                    .deleted
                    .iter()
                    .copied()
                    .filter(|d| {
                        let p = view.parent[d];
                        p == ROOT || view.reachable.contains(&p)
                    })
                    .collect();
                if let Some(node) = self.rng.pick(&restorable) {
                    self.set_deleted(r, node, false);
                    self.stats.restores += 1;
                }
            }
            _ => {
                let rep = &self.replicas[r];
                let now = rep.wall(self.t);
                let expired: Vec<Id> = view.deleted.iter().copied().filter(|d| rep.records[d].deleted.at.wall + RETENTION <= now).collect();
                if let Some(node) = self.rng.pick(&expired) {
                    self.purge(r, node);
                    self.stats.purges += 1;
                }
            }
        }
        // Whatever arrives, a replica's own view stays a tree (`View::derive` asserts it).
        self.observe(r);
    }

    /// Derives replica `r`'s view (which asserts it is a tree) and counts the rules it used.
    fn observe(&mut self, r: usize) {
        let rep = &self.replicas[r];
        let view = rep.view();
        let s = &mut self.stats;
        s.cycles += view.undone.len();
        s.waiting += view.parent.values().filter(|&&p| p == SYNCING).count();
        s.revived += rep.records.iter().filter(|(id, r)| r.deleted.val && !view.deleted.contains(id)).count();
        s.purged_parents += rep.records.values().filter(|r| rep.purged.contains(&r.pos.val.parent)).count();
    }

    fn check_converged(&self) {
        let first = &self.replicas[0];
        for rep in &self.replicas[1..] {
            assert_eq!(rep.records, first.records, "records differ between replicas");
            assert_eq!(rep.purged, first.purged, "purged ids differ between replicas");
        }
        let view = first.view();
        for rep in &self.replicas[1..] {
            assert_eq!(rep.view(), view, "views differ between replicas");
        }
        assert!(view.parent.values().all(|&p| p != SYNCING), "a record still waits for its parent");
        assert!(first.records.keys().all(|id| !first.purged.contains(id)), "a stale record revived a purged one");
        for id in &self.created {
            assert!(first.records.contains_key(id) || first.purged.contains(id), "{id} was lost");
        }
        // A hidden record is explained by a deletion on its path that came after it.
        for (&id, rec) in &first.records {
            if view.reachable.contains(&id) {
                continue;
            }
            let mut n = id;
            let explained = loop {
                if n == ROOT {
                    break false;
                }
                let r = &first.records[&n];
                if r.deleted.val && r.deleted.at > rec.created {
                    break true;
                }
                n = view.parent[&n];
            };
            assert!(explained, "{id} is hidden though nothing above it was deleted after it was created");
        }
    }
}

/// Plays a random history: each step's seed drives that step's choices, so proptest can shrink
/// a failing history by dropping steps and simplifying seeds.
fn run(skews: &[i64], steps: &[u64]) -> Stats {
    let mut sim = Sim::new(0, skews);
    for &seed in steps {
        sim.rng = Rng(seed);
        let dt = sim.rng.below(4);
        sim.advance(dt);
        let n = sim.replicas.len() as u64;
        let a = sim.rng.below(n) as usize;
        if sim.rng.chance(65) {
            sim.random_edit(a);
        } else {
            let b = (a + 1 + sim.rng.below(n - 1) as usize) % n as usize;
            sim.send(a, b, true);
            sim.observe(b);
        }
    }
    sim.sync_all();
    sim.check_converged();
    sim.observe(0);
    sim.stats
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    #[test]
    fn random_histories_converge(
        skews in vec(-15i64..=15, 3),
        steps in vec(any::<u64>(), 1..150),
    ) {
        run(&skews, &steps);
    }
}

/// Fixed histories that must reach every rule, so a weak generator can't pass quietly.
#[test]
fn random_histories_reach_every_rule() {
    let mut total = Stats::default();
    for seed in 0..300 {
        let mut rng = Rng(seed);
        let skews: Vec<i64> = (0..3).map(|_| rng.below(31) as i64 - 15).collect();
        let steps: Vec<u64> = (0..150).map(|_| rng.next()).collect();
        let s = run(&skews, &steps);
        total.cycles += s.cycles;
        total.waiting += s.waiting;
        total.revived += s.revived;
        total.restores += s.restores;
        total.purges += s.purges;
        total.purged_parents += s.purged_parents;
    }
    let Stats { cycles, waiting, revived, restores, purges, purged_parents } = total;
    for (rule, n) in [
        ("cycles", cycles),
        ("waiting", waiting),
        ("revived", revived),
        ("restores", restores),
        ("purges", purges),
        ("purged parents", purged_parents),
    ] {
        assert!(n > 0, "the random histories never hit {rule}");
    }
}

#[test]
fn concurrent_moves_that_close_a_cycle_undo_the_later_move() {
    let mut sim = Sim::new(1, &[0, 0]);
    let a = sim.create(0, ROOT);
    let b = sim.create(0, ROOT);
    sim.sync_all();
    sim.move_to(0, a, b);
    sim.advance(1);
    sim.move_to(1, b, a);
    sim.sync_all();
    sim.check_converged();
    let view = sim.replicas[0].view();
    assert_eq!(view.parent[&a], b, "the earlier move stands");
    assert_eq!(view.parent[&b], ROOT, "the later move goes back where it came from");
}

#[test]
fn a_child_added_under_a_concurrently_deleted_parent_brings_it_back() {
    let mut sim = Sim::new(2, &[0, 0]);
    let p = sim.create(0, ROOT);
    sim.sync_all();
    sim.set_deleted(0, p, true);
    sim.advance(1);
    let c = sim.create(1, p);
    sim.sync_all();
    sim.check_converged();
    let view = sim.replicas[0].view();
    assert!(view.reachable.contains(&p) && view.reachable.contains(&c));
}

#[test]
fn a_child_the_deleting_device_had_seen_stays_in_the_trash() {
    let mut sim = Sim::new(3, &[0, 0]);
    let p = sim.create(0, ROOT);
    let c = sim.create(0, p);
    sim.sync_all();
    sim.advance(1);
    sim.set_deleted(1, p, true);
    sim.sync_all();
    sim.check_converged();
    let view = sim.replicas[0].view();
    assert!(view.deleted.contains(&p));
    assert!(!view.reachable.contains(&p) && !view.reachable.contains(&c));
}

#[test]
fn a_child_added_under_a_purged_parent_shows_at_the_top() {
    let mut sim = Sim::new(4, &[0, 0]);
    let p = sim.create(0, ROOT);
    sim.sync_all();
    sim.set_deleted(0, p, true);
    sim.advance(RETENTION + 1);
    sim.purge(0, p);
    // The other device was away the whole time: it edits the purged node and adds under it.
    sim.edit(1, p);
    let c = sim.create(1, p);
    sim.sync_all();
    sim.check_converged();
    let rep = &sim.replicas[1];
    assert!(rep.purged.contains(&p) && !rep.records.contains_key(&p), "stale edits don't revive a purge");
    let view = rep.view();
    assert_eq!(view.parent[&c], ROOT);
    assert!(view.reachable.contains(&c));
}

#[test]
fn a_record_waits_for_a_parent_that_hasnt_arrived() {
    let mut sim = Sim::new(5, &[0, 0]);
    let p = sim.create(0, ROOT);
    let c = sim.create(0, p);
    let rec = sim.replicas[0].records[&c].clone();
    sim.replicas[1].receive(c, &rec);
    let view = sim.replicas[1].view();
    assert_eq!(view.parent[&c], SYNCING);
    assert!(!view.reachable.contains(&c));
    sim.sync_all();
    let view = sim.replicas[1].view();
    assert_eq!(view.parent[&c], p);
    assert!(view.reachable.contains(&c));
}

/// The limit of comparing clocks: a device whose clock runs slow can add under a parent that
/// another device deleted, and its new child sorts before the deletion. It stays in the Trash
/// with its parent instead of bringing it back. Nothing is lost; restoring the parent shows it.
#[test]
fn a_slow_clock_can_leave_a_concurrent_child_in_the_trash() {
    let mut sim = Sim::new(6, &[0, -30]);
    let p = sim.create(0, ROOT);
    sim.sync_all();
    sim.advance(10);
    sim.set_deleted(0, p, true);
    let c = sim.create(1, p);
    sim.sync_all();
    sim.check_converged();
    let view = sim.replicas[0].view();
    assert!(!view.reachable.contains(&c));
    assert!(sim.replicas[0].records.contains_key(&c));
    sim.set_deleted(0, p, false);
    assert!(sim.replicas[0].view().reachable.contains(&c));
}
