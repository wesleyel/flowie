//! Run in wasm under Node by scripts/wasm-check.sh: two outlines sync through the bodies the
//! server would carry, with an in-memory backend standing in for it.
#![cfg(target_arch = "wasm32")]

use flowie_sync::wire::{ModifyRequest, ModifyResponse};
use flowie_sync::MemoryServer;
use flowie_wasm::Outline;
use wasm_bindgen_test::*;

/// What web/sync.js does, with `server` for fetch.
fn round(outline: &mut Outline, server: &mut MemoryServer) {
    while let Some(body) = outline.next_saves(50) {
        let request: ModifyRequest = serde_json::from_str(&body).unwrap();
        let answer = ModifyResponse { results: server.modify(request.saves) };
        outline.on_saved(&serde_json::to_string(&answer).unwrap()).unwrap();
    }
    let batch = server.changes(outline.cursor().as_deref(), 500);
    outline.on_fetched(&serde_json::to_string(&batch).unwrap()).unwrap();
}

#[wasm_bindgen_test]
fn two_browser_outlines_sync_and_keep_what_they_saved() {
    let mut server = MemoryServer::new();
    let mut a = Outline::open("a.sqlite", "memory").unwrap();
    let mut b = Outline::open("b.sqlite", "memory").unwrap();
    let x = a.create(None, 0, "from a browser").unwrap();
    a.set_text(&x, "from a browser, edited").unwrap();
    round(&mut a, &mut server);
    assert!(!a.has_unsent());
    round(&mut b, &mut server);
    let page: serde_json::Value = serde_json::from_str(&b.page(None, None).unwrap()).unwrap();
    assert_eq!(page[0]["text"], "from a browser, edited");

    b.set_completed(&x, true).unwrap();
    round(&mut b, &mut server);
    round(&mut a, &mut server);
    let page: serde_json::Value = serde_json::from_str(&a.page(None, None).unwrap()).unwrap();
    assert_eq!(page[0]["completed"], true);

    // A failed request leaves its saves to go again.
    a.set_text(&x, "offline edit").unwrap();
    let body = a.next_saves(50).unwrap();
    a.on_save_failed(&body, "offline").unwrap();
    assert!(a.has_unsent());
    round(&mut a, &mut server);
    assert!(!a.has_unsent());
}

#[wasm_bindgen_test]
fn structure_edits_and_breadcrumbs() {
    let mut o = Outline::open("structure.sqlite", "memory").unwrap();
    let a = o.create(None, 0, "a").unwrap();
    let b = o.create_after(&a, "b").unwrap();
    assert!(o.indent(&b).unwrap(), "b goes under a");
    let page: serde_json::Value = serde_json::from_str(&o.page(None, None).unwrap()).unwrap();
    assert_eq!((page[1]["text"].as_str(), page[1]["depth"].as_u64()), (Some("b"), Some(1)));
    let path: serde_json::Value = serde_json::from_str(&o.path(&b).unwrap()).unwrap();
    assert_eq!(path.as_array().unwrap().iter().map(|p| p["text"].as_str().unwrap()).collect::<Vec<_>>(), ["a", "b"]);
    assert!(!o.outdent(&b, Some(a.clone())).unwrap(), "not out of the page it is zoomed into");
    assert!(o.outdent(&b, None).unwrap());
    o.set_collapsed(&a, true).unwrap();
    let page: serde_json::Value = serde_json::from_str(&o.page(None, None).unwrap()).unwrap();
    assert_eq!(page.as_array().unwrap().len(), 2);
    assert_eq!(page[1]["depth"], 0);
}

#[wasm_bindgen_test]
fn search_stars_today_and_undo() {
    let mut o = Outline::open("more.sqlite", "memory").unwrap();
    let a = o.create(None, 0, "groceries").unwrap();
    o.create(Some(a.clone()), 0, "buy #milk").unwrap();
    o.create(None, 1, "work").unwrap();
    let found: serde_json::Value = serde_json::from_str(&o.page(None, Some("#milk".into())).unwrap()).unwrap();
    let texts: Vec<_> = found.as_array().unwrap().iter().map(|r| r["text"].as_str().unwrap()).collect();
    assert_eq!(texts, ["groceries", "buy #milk"], "the match and the bullet above it");

    o.set_starred(&a, true).unwrap();
    let starred: serde_json::Value = serde_json::from_str(&o.starred().unwrap()).unwrap();
    assert_eq!(starred[0]["text"], "groceries");
    let page: serde_json::Value = serde_json::from_str(&o.page(None, None).unwrap()).unwrap();
    assert_eq!(page[0]["starred"], true);
    assert_eq!(page[0]["image"], "", "a bullet that isn't an image shows none");

    let today = o.today();
    assert_eq!(o.today(), today, "the same page each time");
    assert!(o.undo());
    assert!(o.redo());
}
