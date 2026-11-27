//! Two devices syncing through a backend that behaves like CloudKit as CKSyncEngine shows it
//! to `NativeSync` (wiki: Per-Node-Sync §5.2): records by name, an opaque version (the
//! archived system fields) that makes each save conditional, serverRecordChanged with the
//! server's record for a stale one, and a change feed that hands out each record's latest
//! version only. Each device keeps a database, as the apps do.

use std::collections::{BTreeSet, HashMap};

use flowie_core::images::ImageDir;
use flowie_core::journal;
use flowie_core::{NodeId, ROOT};
use flowie_doc::{NativeSaved, NativeSync, Wire, Workspace};

/// Records in one zone of a private database.
#[derive(Default)]
struct FakeCloudKit {
    /// name -> (envelope without etag, change tag, sequence number of the save)
    records: HashMap<String, (String, u64, u64)>,
    seq: u64,
}

impl FakeCloudKit {
    fn system_fields(id: &str, tag: u64) -> String {
        format!("ck:{id}:{tag}")
    }

    fn save(&mut self, wire: Wire) -> NativeSaved {
        let current = self.records.get(&wire.id).map(|(envelope, tag, _)| (envelope.clone(), *tag));
        match current {
            Some((envelope, tag)) if wire.etag.as_deref() != Some(Self::system_fields(&wire.id, tag).as_str()) => {
                NativeSaved::Conflict { server: Wire { etag: Some(Self::system_fields(&wire.id, tag)), id: wire.id, envelope } }
            }
            current => {
                let tag = current.map_or(1, |(_, tag)| tag + 1);
                self.seq += 1;
                self.records.insert(wire.id.clone(), (wire.envelope, tag, self.seq));
                NativeSaved::Saved { etag: Self::system_fields(&wire.id, tag), id: wire.id }
            }
        }
    }

    /// The latest version of every record saved after `token`, and the token to go on from.
    fn changes(&self, token: u64) -> (Vec<Wire>, u64) {
        let mut newer: Vec<_> = self.records.iter().filter(|(_, (_, _, seq))| *seq > token).collect();
        newer.sort_by_key(|(_, (_, _, seq))| *seq);
        let wires = newer
            .iter()
            .map(|(id, (envelope, tag, _))| Wire {
                id: id.to_string(),
                etag: Some(Self::system_fields(id, *tag)),
                envelope: envelope.clone(),
            })
            .collect();
        (wires, self.seq)
    }
}

/// One device: its outline, the engine, and what CKSyncEngine keeps (pending saves, the
/// change token).
struct Device {
    ws: Workspace,
    native: NativeSync,
    pending: BTreeSet<String>,
    token: u64,
}

impl Device {
    fn open(db: &std::path::Path) -> Device {
        let (mut ws, _) = Workspace::open(db, ImageDir::beside(db), |_| {}).unwrap();
        assert!(ws.use_remote("icloud"), "a new outline starts fresh with iCloud");
        Device { ws, native: NativeSync::new(), pending: BTreeSet::new(), token: 0 }
    }

    /// What CKSyncEngine does in one go: send what is pending, in batches of at most 400
    /// (FlowieCloudKit's `nextRecordZoneChangeBatch`), then fetch.
    fn sync(&mut self, cloud: &mut FakeCloudKit) {
        self.ws.persist();
        self.pending.extend(NativeSync::unsent_ids(&self.ws, 10_000));
        while !self.pending.is_empty() {
            let saves = self.native.next_saves(&mut self.ws, 400);
            if saves.is_empty() {
                self.pending.clear();
                break;
            }
            let outcomes: Vec<NativeSaved> = saves.into_iter().map(|w| cloud.save(w)).collect();
            for outcome in &outcomes {
                if let NativeSaved::Saved { id, .. } = outcome {
                    self.pending.remove(id);
                }
            }
            self.native.on_saved(&mut self.ws, outcomes);
            self.pending.extend(NativeSync::unsent_ids(&self.ws, 10_000));
        }
        let (records, token) = cloud.changes(self.token);
        self.token = token;
        self.native.on_fetched(&mut self.ws, &records, "");
    }

    /// The journal pages for `date` under the journal bullet (by text, so a second page with
    /// another id would show).
    fn days(&self, date: &str) -> Vec<NodeId> {
        let Some(journal) = journal::find_journal(&self.ws.tree) else { return Vec::new() };
        self.ws.tree.children(journal).iter().copied().filter(|&id| self.ws.tree.node(id).text == date).collect()
    }

    fn texts(&self, parent: NodeId) -> BTreeSet<String> {
        self.ws.tree.children(parent).iter().map(|&id| self.ws.tree.node(id).text.clone()).collect()
    }
}

fn settle(a: &mut Device, b: &mut Device, cloud: &mut FakeCloudKit) {
    for _ in 0..3 {
        a.sync(cloud);
        b.sync(cloud);
    }
    assert!(NativeSync::unsent_ids(&a.ws, 10).is_empty() && NativeSync::unsent_ids(&b.ws, 10).is_empty());
}

#[test]
fn two_devices_writing_in_todays_journal_offline_make_one_page() {
    let dir = tempfile::tempdir().unwrap();
    let mut cloud = FakeCloudKit::default();
    let mut a = Device::open(&dir.path().join("mac.sqlite"));
    let mut b = Device::open(&dir.path().join("iphone.sqlite"));
    let day = "2026-11-27";

    // Both write in today's journal before either has synced: each makes the page itself.
    a.ws.journal_append(day, "from the Mac").unwrap();
    b.ws.journal_append(day, "from the iPhone\nand a second line").unwrap();
    settle(&mut a, &mut b, &mut cloud);

    for device in [&a, &b] {
        assert_eq!(device.days(day), [journal::day_id(day)], "one page for the day, with the same id");
        assert_eq!(device.ws.tree.children(ROOT).iter().filter(|&&id| id == journal::journal_root_id()).count(), 1);
        let texts = device.texts(journal::day_id(day));
        assert_eq!(texts, ["and a second line", "from the Mac", "from the iPhone"].map(String::from).into());
    }

    // Capturing again on both (quick action, Control Center) reuses the page and its empty row.
    let next = "2026-11-28";
    let ca = a.ws.journal_capture(next);
    let cb = b.ws.journal_capture(next);
    assert_eq!((ca.node, cb.node), (journal::day_blank_id(next), journal::day_blank_id(next)));
    settle(&mut a, &mut b, &mut cloud);
    for device in [&a, &b] {
        assert_eq!(device.days(next), [journal::day_id(next)]);
        assert_eq!(device.ws.tree.children(journal::day_id(next)), [journal::day_blank_id(next)], "one blank first line");
    }
}

#[test]
fn a_device_with_an_outline_sends_all_of_it_when_it_starts_with_icloud() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("mac.sqlite");
    let mut cloud = FakeCloudKit::default();
    // An outline written before sync was on, then synced with a server: nothing is waiting.
    {
        let (mut ws, _) = Workspace::open(&db, ImageDir::beside(&db), |_| {}).unwrap();
        for i in 0..450 {
            ws.tree.create_at(ROOT, i, &format!("bullet {i}"));
        }
        ws.commit_with_imports(None, None, None);
        ws.use_remote("https://sync.example.com#outline");
        flowie_sync::RecordStore::set_cursor(&mut ws, "42".into());
        let all: Vec<_> = flowie_sync::RecordStore::unsent(&ws, 1000);
        for id in all {
            flowie_sync::RecordStore::clear_unsent(&mut ws, id);
        }
        ws.flush().unwrap();
    }
    let (ws, _) = Workspace::open(&db, ImageDir::beside(&db), |_| {}).unwrap();
    assert!(!ws.sync.has_unsent());
    let mut mac = Device { ws, native: NativeSync::new(), pending: BTreeSet::new(), token: 0 };
    assert!(mac.ws.use_remote("icloud"), "iCloud after a server is a fresh start");
    mac.sync(&mut cloud);
    assert_eq!(cloud.records.len(), 450, "the whole outline goes up, in batches of 400");

    let mut iphone = Device::open(&dir.path().join("iphone.sqlite"));
    iphone.sync(&mut cloud);
    assert_eq!(iphone.ws.tree.children(ROOT).len(), 450);

    // Opened again, iCloud carries on where it was: nothing is sent again.
    mac.ws.flush().unwrap();
    drop(mac);
    let (mut ws, _) = Workspace::open(&db, ImageDir::beside(&db), |_| {}).unwrap();
    assert!(!ws.use_remote("icloud"));
    assert!(!ws.sync.has_unsent());
}
