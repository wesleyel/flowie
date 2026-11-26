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
    let page: serde_json::Value = serde_json::from_str(&b.page(None).unwrap()).unwrap();
    assert_eq!(page[0]["text"], "from a browser, edited");

    b.set_completed(&x, true).unwrap();
    round(&mut b, &mut server);
    round(&mut a, &mut server);
    let page: serde_json::Value = serde_json::from_str(&a.page(None).unwrap()).unwrap();
    assert_eq!(page[0]["completed"], true);

    // A failed request leaves its saves to go again.
    a.set_text(&x, "offline edit").unwrap();
    let body = a.next_saves(50).unwrap();
    a.on_save_failed(&body, "offline").unwrap();
    assert!(a.has_unsent());
    round(&mut a, &mut server);
    assert!(!a.has_unsent());
}
