//! The server over real HTTP, driven by the desktop's HTTP driver.

use std::time::{Duration, Instant};

use flowie_core::tree::{Tree, ROOT};
use flowie_sync::http::HttpBackend;
use flowie_sync::{sync_once, Backend, Engine, MemoryStore};
use sha2::{Digest, Sha256};

/// A server on a free local port, for the rest of the test process. Returns its URL and a
/// device token.
fn start() -> (String, String) {
    let dir = tempfile::tempdir().unwrap().keep();
    let conn = flowie_server::open(&dir.join("server.sqlite")).unwrap();
    let token = flowie_server::add_token(&conn, "test device").unwrap();
    let state = flowie_server::AppState::new(conn, dir.join("blobs"));
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            tx.send(listener.local_addr().unwrap()).unwrap();
            flowie_server::serve(listener, state).await.unwrap();
        });
    });
    (format!("http://{}", rx.recv().unwrap()), token)
}

fn device(actor: u64) -> (MemoryStore, Engine) {
    let mut tree = Tree::new();
    tree.set_actor(actor);
    (MemoryStore::new(tree), Engine::new())
}

#[test]
fn requests_need_a_known_device_token() {
    let (url, token) = start();
    assert_eq!(ureq::get(&format!("{url}/v1/health")).call().unwrap().into_string().unwrap(), "ok");
    let changes = format!("{url}/v1/zones/outline/changes");
    assert!(matches!(ureq::get(&changes).call(), Err(ureq::Error::Status(401, _))));
    assert!(matches!(ureq::get(&changes).set("Authorization", "Bearer nope").call(), Err(ureq::Error::Status(401, _))));
    assert!(ureq::get(&changes).set("Authorization", &format!("Bearer {token}")).call().is_ok());
    let mut bad_zone = HttpBackend::new(&url, "no/such", &token);
    assert!(bad_zone.changes(None, 10).is_err());
}

#[test]
fn two_devices_sync_through_the_server() {
    let (url, token) = start();
    let ((mut a, mut ea), (mut b, mut eb)) = (device(1), device(2));
    let (mut ha, mut hb) = (HttpBackend::new(&url, "outline", &token), HttpBackend::new(&url, "outline", &token));

    let x = a.tree.create_at(ROOT, 0, "x");
    let y = a.tree.create_at(x, 0, "y");
    a.collect();
    sync_once(&mut ea, &mut a, &mut ha, 50).unwrap();
    let report = sync_once(&mut eb, &mut b, &mut hb, 50).unwrap();
    assert_eq!(report.received, 2);
    assert_eq!(b.tree.descendants(ROOT), [x, y]);

    // Both edit before syncing again: b's save of x conflicts, and both edits survive.
    a.tree.set_text(x, "a's text");
    a.collect();
    b.tree.set_note(x, "b's note");
    b.tree.move_to(y, ROOT, 1);
    b.collect();
    sync_once(&mut ea, &mut a, &mut ha, 50).unwrap();
    for _ in 0..2 {
        sync_once(&mut eb, &mut b, &mut hb, 50).unwrap();
        sync_once(&mut ea, &mut a, &mut ha, 50).unwrap();
    }
    for store in [&a, &b] {
        let node = store.tree.node(x);
        assert_eq!((node.text.as_str(), node.note.as_str()), ("a's text", "b's note"));
        assert_eq!(store.tree.parent(y), ROOT);
        assert!(!store.has_unsent());
    }
}

#[test]
fn a_pull_waits_for_a_save() {
    let (url, token) = start();
    let mut waiting = HttpBackend::new(&url, "waits", &token);
    waiting.wait = Duration::from_secs(20);
    let (url2, token2) = (url.clone(), token.clone());
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        let (mut store, mut engine) = device(7);
        store.tree.create_at(ROOT, 0, "late");
        store.collect();
        sync_once(&mut engine, &mut store, &mut HttpBackend::new(&url2, "waits", &token2), 50).unwrap();
    });
    let started = Instant::now();
    let batch = waiting.changes(None, 10).unwrap();
    assert_eq!(batch.records.len(), 1, "it came while the pull waited");
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn image_files_are_checked_against_their_names() {
    let (url, token) = start();
    let auth = format!("Bearer {token}");
    let bytes = b"GIF89a\x01\x00\x01\x00";
    let digest: String = Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect();
    let blob = format!("{url}/v1/blobs/{digest}.gif");
    assert!(matches!(ureq::head(&blob).set("Authorization", &auth).call(), Err(ureq::Error::Status(404, _))));
    let wrong = format!("{url}/v1/blobs/{}.gif", "0".repeat(64));
    assert!(matches!(ureq::put(&wrong).set("Authorization", &auth).send_bytes(bytes), Err(ureq::Error::Status(400, _))));
    assert_eq!(ureq::put(&blob).set("Authorization", &auth).send_bytes(bytes).unwrap().status(), 201);
    assert_eq!(ureq::head(&blob).set("Authorization", &auth).call().unwrap().status(), 200);
    let mut back = Vec::new();
    std::io::Read::read_to_end(&mut ureq::get(&blob).set("Authorization", &auth).call().unwrap().into_reader(), &mut back).unwrap();
    assert_eq!(back, bytes);
}
