//! A save never drops a write the backend had. In its own test binary: it sets the process's
//! clock, which would disturb tests running beside it.

use std::sync::atomic::{AtomicI64, Ordering};

use flowie_core::tree::{set_clock, Tree, ROOT};
use flowie_sync::{Engine, MemoryServer, MemoryStore, RecordStore};

static NOW: AtomicI64 = AtomicI64::new(0);

fn now() -> i64 {
    NOW.load(Ordering::SeqCst)
}

fn at(millis: i64) {
    NOW.store(millis, Ordering::SeqCst);
}

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

    fn push(&mut self, server: &mut MemoryServer) {
        self.store.collect();
        loop {
            let saves = self.engine.next_saves(&self.store, 50);
            if saves.is_empty() {
                break;
            }
            let results = server.modify(saves);
            self.engine.on_saved(&mut self.store, results);
        }
    }

    fn pull(&mut self, server: &MemoryServer) {
        let batch = server.changes(self.store.cursor().as_deref(), 1000);
        self.engine.on_fetched(&mut self.store, batch);
        self.store.collect();
    }
}

#[test]
fn removing_a_record_keeps_what_the_backend_had_for_a_later_revival() {
    set_clock(Some(now));
    let (mut a, mut b, mut c) = (Device::new(1), Device::new(2), Device::new(3));
    let mut server = MemoryServer::new();

    at(1000);
    let x = a.store.tree.create_at(ROOT, 0, "v0");
    a.push(&mut server);
    b.pull(&server);
    c.pull(&server);

    at(2000);
    b.store.tree.set_text(x, "b's text");
    b.push(&mut server);

    // `a` sees b's text, then removes the bullet for good (merging it into another, say).
    at(3000);
    a.pull(&server);
    a.store.tree.remove(x);
    a.push(&mut server);

    // `c` never saw either, and checks the bullet off later: a write after the removal, which
    // brings it back. It must come back with b's text, which the backend had before a's save.
    at(4000);
    c.store.tree.set_completed(x, true);
    c.push(&mut server);
    c.pull(&server);

    for _ in 0..3 {
        for dev in [&mut a, &mut b, &mut c] {
            dev.push(&mut server);
            dev.pull(&server);
        }
    }
    for dev in [&a, &b, &c] {
        assert!(dev.store.tree.contains(x));
        assert_eq!(dev.store.tree.node(x).text, "b's text");
        assert!(dev.store.tree.node(x).completed);
    }
    set_clock(None);
}
