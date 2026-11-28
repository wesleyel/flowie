//! Two devices syncing through a backend that behaves like CloudKit as CKSyncEngine shows it
//! to `NativeSync` (wiki: Per-Node-Sync §5.2): records by name, an opaque version (the
//! archived system fields) that makes each save conditional, serverRecordChanged with the
//! server's record for a stale one, and a change feed that hands out each record's latest
//! version only. Each device keeps a database, as the apps do.

use std::collections::{BTreeSet, HashMap};

use flowie_core::images::ImageDir;
use flowie_core::journal;
use flowie_core::{NodeId, ROOT};
use flowie_doc::{FileTaken, NativeSaved, NativeSync, Wire, WireFile, Workspace};

/// What one CKRecord holds: a `Record`'s envelope, or a `Blob`'s file (its CKAsset) and the
/// fields beside it.
#[derive(Clone)]
enum Stored {
    Record(String),
    Blob(Vec<u8>, u32, u32, String),
}

/// Records in one zone of a private database.
struct FakeCloudKit {
    /// name -> (what it holds, change tag, sequence number of the save)
    records: HashMap<String, (Stored, u64, u64)>,
    seq: u64,
    /// The names saved, in order.
    log: Vec<String>,
    /// Where fetched assets are downloaded, as CKSyncEngine does before handing a record over.
    downloads: tempfile::TempDir,
}

impl Default for FakeCloudKit {
    fn default() -> Self {
        FakeCloudKit { records: HashMap::new(), seq: 0, log: Vec::new(), downloads: tempfile::tempdir().unwrap() }
    }
}

impl FakeCloudKit {
    fn system_fields(id: &str, tag: u64) -> String {
        format!("ck:{id}:{tag}")
    }

    fn save(&mut self, wire: Wire) -> NativeSaved {
        let current = self.records.get(&wire.id).map(|(_, tag, _)| *tag);
        if current.is_some_and(|tag| wire.etag.as_deref() != Some(Self::system_fields(&wire.id, tag).as_str())) {
            return NativeSaved::Conflict { server: self.wire(&wire.id).expect("saved") };
        }
        let stored = match &wire.file {
            // The asset is uploaded from the file as it is now.
            Some(file) => Stored::Blob(std::fs::read(&file.path).unwrap(), file.width, file.height, file.mime.clone()),
            None => Stored::Record(wire.envelope),
        };
        let tag = current.map_or(1, |tag| tag + 1);
        self.seq += 1;
        self.log.push(wire.id.clone());
        self.records.insert(wire.id.clone(), (stored, tag, self.seq));
        NativeSaved::Saved { etag: Self::system_fields(&wire.id, tag), id: wire.id }
    }

    /// A record as FlowieCloudKit hands it over: a `Blob`'s asset downloaded to a file.
    fn wire(&self, id: &str) -> Option<Wire> {
        let (stored, tag, _) = self.records.get(id)?;
        let etag = Some(Self::system_fields(id, *tag));
        Some(match stored {
            Stored::Record(envelope) => Wire { id: id.to_string(), etag, envelope: envelope.clone(), file: None },
            Stored::Blob(bytes, width, height, mime) => {
                let path = self.downloads.path().join(format!("asset-{id}"));
                std::fs::write(&path, bytes).unwrap();
                let file = WireFile { path: path.to_string_lossy().into_owned(), width: *width, height: *height, mime: mime.clone() };
                Wire { id: id.to_string(), etag: None, envelope: String::new(), file: Some(file) }
            }
        })
    }

    /// The latest version of every record saved after `token`, and the token to go on from.
    fn changes(&self, token: u64) -> (Vec<Wire>, u64) {
        let mut newer: Vec<_> = self.records.iter().filter(|(_, (_, _, seq))| *seq > token).collect();
        newer.sort_by_key(|(_, (_, _, seq))| *seq);
        let wires = newer.iter().filter_map(|(id, _)| self.wire(id)).collect();
        (wires, self.seq)
    }

    fn envelope(&self, id: &str) -> String {
        match &self.records[id].0 {
            Stored::Record(envelope) => envelope.clone(),
            Stored::Blob(..) => panic!("{id} is a Blob"),
        }
    }
}

/// One device: its outline, the engine, and what CKSyncEngine keeps (pending saves, the
/// change token).
struct Device {
    ws: Workspace,
    native: NativeSync,
    pending: BTreeSet<String>,
    token: u64,
    /// Records per batch (FlowieCloudKit sends at most 400).
    batch: usize,
}

impl Device {
    fn open(db: &std::path::Path) -> Device {
        let (mut ws, _) = Workspace::open(db, ImageDir::beside(db), |_| {}).unwrap();
        assert!(ws.use_remote("icloud"), "a new outline starts fresh with iCloud");
        Device { ws, native: NativeSync::new(), pending: BTreeSet::new(), token: 0, batch: 400 }
    }

    /// What CKSyncEngine does in one go: send what is pending, in batches of at most 400
    /// (FlowieCloudKit's `nextRecordZoneChangeBatch`), then fetch.
    fn sync(&mut self, cloud: &mut FakeCloudKit) {
        self.ws.persist();
        self.pending.extend(self.native.to_send(&self.ws, 10_000));
        while !self.pending.is_empty() {
            let saves = self.native.next_saves(&mut self.ws, self.batch);
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
            self.pending.extend(self.native.to_send(&self.ws, 10_000));
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
    let mut mac = Device { ws, native: NativeSync::new(), pending: BTreeSet::new(), token: 0, batch: 400 };
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

/// A later version may put new kinds of records in the zone (image files as `Blob`, wiki:
/// Per-Node-Sync §3.5). FlowieCloudKit hands over only `Record`s and never a deletion; inside
/// a `Record`, an envelope this version can't read must leave the bullet whose name it
/// carries alone: not removed, not overwritten, and not sent back as if it were.
#[test]
fn a_record_this_version_cant_read_never_touches_a_bullet() {
    let dir = tempfile::tempdir().unwrap();
    let mut cloud = FakeCloudKit::default();
    let mut mac = Device::open(&dir.path().join("mac.sqlite"));
    let id = mac.ws.tree.create_at(ROOT, 0, "keep me");
    mac.ws.commit_with_imports(None, None, None);
    mac.sync(&mut cloud);
    let name = id.to_string();
    let sent = cloud.envelope(&name);

    // The bullet's own record, changed the way a later version might: another kind (with a
    // removal in it), encrypted, or not JSON at all.
    let mut envelope: serde_json::Value = serde_json::from_str(&sent).unwrap();
    envelope["payload"]["removed"] = "ffffffffffff-0000-ffffffffffffffff".into();
    let mut blob = envelope.clone();
    blob["kind"] = "blob".into();
    let mut encrypted = envelope.clone();
    encrypted["enc"] = serde_json::json!({"alg": "later", "key_id": "k", "nonce": "n"});
    let unreadable = [blob.to_string(), encrypted.to_string(), "not json".to_string()];
    let wires: Vec<Wire> = unreadable
        .iter()
        .enumerate()
        .map(|(i, envelope)| Wire { id: name.clone(), etag: Some(format!("later:{i}")), envelope: envelope.clone(), file: None })
        .collect();

    assert!(!mac.native.on_fetched(&mut mac.ws, &wires, "").changed, "nothing changes");
    assert_eq!(mac.ws.tree.children(ROOT), [id]);
    assert_eq!(mac.ws.tree.node(id).text, "keep me");
    assert!(NativeSync::unsent_ids(&mac.ws, 10).is_empty(), "and nothing is sent back");

    // A conflict answered with such a record: the bullet stays as it is here and goes out
    // again later.
    mac.ws.tree.set_text(id, "edited");
    mac.ws.commit_with_imports(None, None, None);
    let saves = mac.native.next_saves(&mut mac.ws, 10);
    assert_eq!(saves.len(), 1);
    mac.native.on_saved(&mut mac.ws, wires.into_iter().map(|server| NativeSaved::Conflict { server }).collect());
    assert_eq!(mac.ws.tree.node(id).text, "edited");
    assert_eq!(NativeSync::unsent_ids(&mac.ws, 10), std::slice::from_ref(&name));

    // The same removal as a bullet's record does remove it: the ones above were ignored for
    // what they are, not for what they carry.
    let removal = Wire { id: name, etag: Some("now".into()), envelope: envelope.to_string(), file: None };
    assert!(mac.native.on_fetched(&mut mac.ws, &[removal], "").changed);
    assert!(mac.ws.tree.children(ROOT).is_empty());
}

/// An image row added on the Mac, as a desktop does: the file in the image folder, noted in
/// `blobs`, and the row pointing at it.
fn add_image(device: &mut Device, bytes: &[u8], at: usize) -> (NodeId, String) {
    let info = device.ws.image_dir.as_ref().unwrap().put(bytes).unwrap();
    let name = info.name.clone();
    device.ws.record_blob(info);
    let row = device.ws.tree.create_at(ROOT, at, "a picture");
    let mut node = device.ws.tree.node(row).clone();
    node.kind = flowie_core::BlockKind::Image;
    node.image = name.clone();
    device.ws.tree.put(node);
    device.ws.commit_with_imports(None, None, None);
    (row, name)
}

/// A GIF of `w`×`h` pixels (and `salt`, to make files that differ).
fn gif(w: u8, h: u8, salt: u8) -> Vec<u8> {
    vec![b'G', b'I', b'F', b'8', b'9', b'a', w, 0, h, 0, salt]
}

#[test]
fn an_image_goes_up_before_its_row_and_arrives_with_its_size() {
    let dir = tempfile::tempdir().unwrap();
    let mut cloud = FakeCloudKit::default();
    let mut mac = Device::open(&dir.path().join("mac.sqlite"));
    let mut iphone = Device::open(&dir.path().join("iphone.sqlite"));
    let bytes = gif(2, 3, 0);
    let (row, name) = add_image(&mut mac, &bytes, 0);
    mac.sync(&mut cloud);

    let at = |id: &str| cloud.log.iter().position(|saved| saved == id).unwrap();
    assert!(at(&flowie_sync::blob_name(&name)) < at(&row.to_string()), "the file is saved before the row");
    assert!(matches!(&cloud.records[&name].0, Stored::Blob(saved, 2, 3, mime) if *saved == bytes && mime == "image/gif"));

    iphone.sync(&mut cloud);
    assert_eq!(iphone.ws.tree.node(row).image, name);
    assert_eq!(iphone.ws.image_dir.as_ref().unwrap().read(&name).unwrap(), bytes, "the file came with the row");
    let known = &iphone.ws.blobs[&name];
    assert_eq!((known.width, known.height), (2, 3), "and its size, for the row's height");

    // Edited again, the row goes alone: the file is there already.
    mac.ws.tree.set_text(row, "a caption");
    mac.ws.commit_with_imports(None, None, None);
    let saves = mac.native.next_saves(&mut mac.ws, 10);
    assert_eq!(saves.iter().map(|w| w.id.clone()).collect::<Vec<_>>(), [row.to_string()]);

    // The iPhone sends the row back after an edit; the file it got isn't sent again either.
    iphone.ws.tree.set_text(row, "from the iPhone");
    iphone.ws.commit_with_imports(None, None, None);
    let saves = iphone.native.next_saves(&mut iphone.ws, 10);
    assert!(saves.iter().all(|w| w.file.is_none()), "the iPhone knows the file is in iCloud");
}

#[test]
fn a_row_that_comes_before_its_file_shows_it_once_the_file_arrives() {
    let dir = tempfile::tempdir().unwrap();
    let mut mac = Device::open(&dir.path().join("mac.sqlite"));
    let mut iphone = Device::open(&dir.path().join("iphone.sqlite"));
    let bytes = gif(4, 5, 1);
    let (row, name) = add_image(&mut mac, &bytes, 0);
    let saves = mac.native.next_saves(&mut mac.ws, 10);
    let (file, record): (Vec<Wire>, Vec<Wire>) = saves.into_iter().partition(|w| w.file.is_some());
    assert_eq!(file.len(), 1);
    let file = file.into_iter().next().unwrap();
    assert_eq!(file.file.as_ref().map(|f| (f.width, f.height)), Some((4, 5)));

    // The row first, in one fetch: a placeholder of unknown size.
    assert!(iphone.native.on_fetched(&mut iphone.ws, &record, "").changed);
    assert!(iphone.ws.tree.contains(row) && !iphone.ws.blobs.contains_key(&name));
    // Then the file: the outline says something changed, so the row is shown again.
    assert!(iphone.native.on_fetched(&mut iphone.ws, std::slice::from_ref(&file), "").changed, "a row waits for it");
    assert_eq!(iphone.ws.image_dir.as_ref().unwrap().read(&name).unwrap(), bytes);
    assert_eq!((iphone.ws.blobs[&name].width, iphone.ws.blobs[&name].height), (4, 5));
    assert!(!iphone.native.on_fetched(&mut iphone.ws, &[file], "").changed, "the same file again changes nothing");
}

#[test]
fn a_file_that_isnt_what_its_name_says_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let mut iphone = Device::open(&dir.path().join("iphone.sqlite"));
    let real = flowie_core::images::info(&gif(2, 2, 2)).unwrap().name;
    let path = dir.path().join("download");
    let other = flowie_core::images::info(&gif(2, 2, 3)).unwrap().name;
    for (bytes, why) in [(gif(2, 2, 3), FileTaken::OtherName(other)), (b"not an image".to_vec(), FileTaken::NotAnImage(12))] {
        std::fs::write(&path, bytes).unwrap();
        let file = WireFile { path: path.to_string_lossy().into_owned(), width: 2, height: 2, mime: "image/gif".into() };
        let wire = Wire { id: flowie_sync::blob_name(&real), etag: None, envelope: String::new(), file: Some(file) };
        let fetched = iphone.native.on_fetched(&mut iphone.ws, &[wire], "");
        assert!(!fetched.changed);
        assert_eq!(fetched.files, [(real.clone(), why)], "and says why, for the sync log");
    }
    let missing = WireFile { path: dir.path().join("gone").to_string_lossy().into_owned(), width: 0, height: 0, mime: String::new() };
    let wire = Wire { id: real.clone(), etag: None, envelope: String::new(), file: Some(missing) };
    let fetched = iphone.native.on_fetched(&mut iphone.ws, &[wire], "");
    assert!(!fetched.changed);
    assert!(matches!(&fetched.files[..], [(_, FileTaken::Unreadable(_))]));
    assert!(iphone.ws.blobs.is_empty() && !iphone.ws.image_dir.as_ref().unwrap().has(&real));
    assert!(iphone.ws.tree.children(ROOT).is_empty(), "and no bullet comes of it");
}

#[test]
fn a_file_whose_save_failed_goes_again_on_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let mut cloud = FakeCloudKit::default();
    let mut mac = Device::open(&dir.path().join("mac.sqlite"));
    let (row, name) = add_image(&mut mac, &gif(3, 3, 4), 0);
    let remote = flowie_sync::blob_name(&name);
    let saves = mac.native.next_saves(&mut mac.ws, 10);
    assert_eq!(saves.iter().map(|w| w.id.clone()).collect::<Vec<_>>(), [remote.clone(), row.to_string()]);
    // The asset didn't upload; the row was saved.
    let outcomes = saves
        .into_iter()
        .map(|w| match w.file {
            Some(_) => NativeSaved::Failed { id: w.id, error: "network".into() },
            None => cloud.save(w),
        })
        .collect();
    mac.native.on_saved(&mut mac.ws, outcomes);
    assert_eq!(mac.native.to_send(&mac.ws, 10), std::slice::from_ref(&remote), "only the file waits to be sent");
    mac.sync(&mut cloud);
    assert!(matches!(cloud.records[&remote].0, Stored::Blob(..)));
    assert!(mac.native.to_send(&mac.ws, 10).is_empty());

    let mut iphone = Device::open(&dir.path().join("iphone.sqlite"));
    iphone.sync(&mut cloud);
    assert!(iphone.ws.image_dir.as_ref().unwrap().has(&name));
}

#[test]
fn files_already_in_icloud_and_full_batches_still_go_before_their_rows() {
    let dir = tempfile::tempdir().unwrap();
    let mut cloud = FakeCloudKit::default();
    let mut mac = Device::open(&dir.path().join("mac.sqlite"));
    let mut rows = Vec::new();
    for i in 0..5 {
        rows.push(add_image(&mut mac, &gif(1, 1, 10 + i), i as usize));
    }
    // The same picture twice: one file.
    let first = rows[0].1.clone();
    let (twin, _) = {
        let info = mac.ws.blobs[&first].clone();
        let row = mac.ws.tree.create_at(ROOT, 5, "again");
        let mut node = mac.ws.tree.node(row).clone();
        node.kind = flowie_core::BlockKind::Image;
        node.image = info.name.clone();
        mac.ws.tree.put(node);
        mac.ws.commit_with_imports(None, None, None);
        (row, info.name)
    };
    // Another Mac put one of the files there already (a fresh start sends everything again).
    let other = flowie_sync::blob_name(&rows[1].1);
    let path = mac.ws.image_dir.as_ref().unwrap().path(&rows[1].1).unwrap();
    let file = WireFile { path: path.to_string_lossy().into_owned(), width: 1, height: 1, mime: "image/gif".into() };
    cloud.save(Wire { id: other.clone(), etag: None, envelope: String::new(), file: Some(file) });

    mac.batch = 3;
    mac.sync(&mut cloud);
    for (row, name) in &rows {
        let at = |id: &str| cloud.log.iter().position(|saved| saved == id).unwrap();
        assert!(at(&flowie_sync::blob_name(name)) < at(&row.to_string()), "{name} before its row");
    }
    assert!(cloud.log.contains(&twin.to_string()));
    assert_eq!(cloud.log.iter().filter(|id| **id == other).count(), 1, "a file there already isn't saved again");
    assert_eq!(cloud.log.iter().filter(|id| **id == first).count(), 1, "nor one two rows show");
    assert!(mac.native.to_send(&mac.ws, 10).is_empty());

    let mut iphone = Device::open(&dir.path().join("iphone.sqlite"));
    iphone.sync(&mut cloud);
    for (_, name) in &rows {
        assert!(iphone.ws.image_dir.as_ref().unwrap().has(name));
    }
}

/// The iPad's case: a device already syncing gets a row in an incremental fetch, but its file
/// doesn't come with it (CKSyncEngine handed the `Blob` over without its download, or an older
/// version ignored the record). The change token moves past both, so only asking iCloud for
/// the file by name brings it: the outline says which files it shows and doesn't have.
#[test]
fn a_file_that_didnt_come_with_its_row_is_asked_for_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let mut cloud = FakeCloudKit::default();
    let mut mac = Device::open(&dir.path().join("mac.sqlite"));
    let mut ipad = Device::open(&dir.path().join("ipad.sqlite"));
    mac.sync(&mut cloud);
    ipad.sync(&mut cloud);
    assert!(NativeSync::missing_files(&ipad.ws, 10).is_empty());

    let bytes = gif(7, 3, 9);
    let (row, name) = add_image(&mut mac, &bytes, 0);
    assert!(NativeSync::missing_files(&mac.ws, 10).is_empty(), "the Mac has its file");
    mac.sync(&mut cloud);

    // The incremental fetch: the row arrives, the Blob without a file to read.
    let (mut records, token) = cloud.changes(ipad.token);
    ipad.token = token;
    let blob = records.iter_mut().find(|w| w.file.is_some()).unwrap();
    blob.file.as_mut().unwrap().path = dir.path().join("never downloaded").to_string_lossy().into_owned();
    let fetched = ipad.native.on_fetched(&mut ipad.ws, &records, "");
    assert!(matches!(&fetched.files[..], [(id, FileTaken::Unreadable(_))] if *id == name), "the log says why");
    assert_eq!(ipad.ws.tree.node(row).image, name);
    ipad.sync(&mut cloud);
    assert!(ipad.ws.blobs.is_empty(), "the next fetch doesn't bring it again");

    let missing = NativeSync::missing_files(&ipad.ws, 10);
    assert_eq!(missing, [flowie_sync::blob_name(&name)]);
    let asked: Vec<Wire> = missing.iter().filter_map(|id| cloud.wire(id)).collect();
    let fetched = ipad.native.on_fetched(&mut ipad.ws, &asked, "");
    assert!(fetched.changed && fetched.files == [(name.clone(), FileTaken::Kept)], "the row is shown again");
    assert_eq!(ipad.ws.image_dir.as_ref().unwrap().read(&name).unwrap(), bytes);
    assert!(NativeSync::missing_files(&ipad.ws, 10).is_empty());
}
