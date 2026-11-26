//! Two Flowie outlines, each in its own database, syncing through one server: what the app
//! will do (P2 in docs/roadmap/per-node-sync.md), short of its sync thread.

use std::path::Path;

use flowie_core::images::ImageDir;
use flowie_core::ROOT;
use flowie_doc::Workspace;
use flowie_sync::http::HttpBackend;
use flowie_sync::{sync_once, Engine, SyncReport};

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

fn open(db: &Path) -> Workspace {
    let (mut ws, _) = Workspace::open(db, ImageDir::beside(db), |_| {}).unwrap();
    ws.set_sync_remote("home server");
    ws
}

fn sync(ws: &mut Workspace, backend: &mut HttpBackend) -> SyncReport {
    sync_once(&mut Engine::new(), ws, backend, 50).unwrap()
}

#[test]
fn two_outlines_sync_through_the_server_across_restarts() {
    let (url, token) = start();
    let dir = tempfile::tempdir().unwrap();
    let (db_a, db_b) = (dir.path().join("a.sqlite"), dir.path().join("b.sqlite"));
    let mut server_a = HttpBackend::new(&url, "outline", &token);
    let mut server_b = HttpBackend::new(&url, "outline", &token);

    let mut a = open(&db_a);
    let x = a.tree.create_at(ROOT, 0, "x");
    let y = a.tree.create_at(x, 0, "y");
    a.commit_with_imports(None, None, None);
    assert_eq!(sync(&mut a, &mut server_a).sent, 2);

    let mut b = open(&db_b);
    let welcome = b.tree.create_at(ROOT, 0, "b's own");
    b.commit_with_imports(None, None, None);
    sync(&mut b, &mut server_b);
    assert_eq!(b.tree.descendants(ROOT).len(), 3);
    assert_eq!(b.tree.parent(y), x);

    // Offline on both: b edits x and quits before syncing; a deletes y.
    b.tree.set_text(x, "x, edited on b");
    b.commit_typing(x, None, None);
    b.flush().unwrap();
    drop(b);
    a.tree.trash(y);
    a.commit_with_imports(None, None, None);
    sync(&mut a, &mut server_a);

    // b starts again: its unsent edit and where it left off pulling were saved.
    let mut b = open(&db_b);
    let report = sync(&mut b, &mut server_b);
    assert_eq!(report.sent, 1, "only the edit made while offline");
    assert_eq!(report.received, 1, "only what changed since b last pulled");
    assert!(report.settled);
    sync(&mut a, &mut server_a);

    for ws in [&a, &b] {
        assert_eq!(ws.tree.node(x).text, "x, edited on b");
        assert!(!ws.tree.contains(y) && ws.tree.trashed() == [y]);
        assert!(ws.tree.contains(welcome));
    }

    // Both saved what they merged: a restart shows the same outline.
    a.flush().unwrap();
    b.flush().unwrap();
    let outline = |ws: &Workspace| ws.tree.descendants(ROOT).iter().map(|&id| ws.tree.node(id).text.clone()).collect::<Vec<_>>();
    let (before_a, before_b) = (outline(&a), outline(&b));
    drop((a, b));
    assert_eq!(outline(&open(&db_a)), before_a);
    assert_eq!(outline(&open(&db_b)), before_b);
    assert_eq!(before_a, before_b);
}
