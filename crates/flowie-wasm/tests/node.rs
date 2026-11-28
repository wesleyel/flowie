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

fn value(json: &str) -> serde_json::Value {
    serde_json::from_str(json).unwrap()
}

fn texts(o: &mut Outline, root: Option<String>) -> Vec<String> {
    value(&o.page(root, None).unwrap()).as_array().unwrap().iter().map(|r| r["text"].as_str().unwrap().to_string()).collect()
}

#[wasm_bindgen_test]
fn return_and_backspace_split_and_merge_in_utf16() {
    let mut o = Outline::open("keys.sqlite", "memory").unwrap();
    let a = o.create(None, 0, "日😀 world").unwrap();
    // After 日😀: three UTF-16 units, as JavaScript counts them.
    let out = value(&o.enter(&a, 3, 3, None).unwrap());
    assert_eq!(out["structure_changed"], true);
    let focus = &out["focus"];
    assert_eq!((focus["start"].as_u64(), focus["end"].as_u64()), (Some(0), Some(0)));
    assert_eq!(texts(&mut o, None), ["日😀", " world"]);
    let back = value(&o.backspace_at_start(focus["id"].as_str().unwrap(), None).unwrap());
    assert_eq!((back["focus"]["id"].as_str(), back["focus"]["start"].as_u64()), (Some(a.as_str()), Some(3)));
    assert_eq!(texts(&mut o, None), ["日😀 world"]);
    assert!(o.can_undo() && !o.can_redo());
    assert!(o.undo());
    assert!(o.can_redo());
}

#[wasm_bindgen_test]
fn the_first_clients_calls_also_leave_their_outcome() {
    let mut o = Outline::open("outcome.sqlite", "memory").unwrap();
    let a = o.create(None, 0, "a").unwrap();
    assert_eq!(value(&o.last_outcome())["focus"]["id"], a.as_str());
    let b = o.create_after(&a, "b").unwrap();
    assert!(o.indent(&b).unwrap());
    assert_eq!(value(&o.last_outcome())["structure_changed"], true);
    o.set_text(&b, "typed").unwrap();
    let out = value(&o.last_outcome());
    assert_eq!((out["changed_ids"][0].as_str(), out["structure_changed"].as_bool()), (Some(b.as_str()), Some(false)));
    let rows = value(&o.rows(vec![b.clone()], None).unwrap());
    assert_eq!((rows[0]["text"].as_str(), rows[0]["depth"].as_u64()), (Some("typed"), Some(1)));
    assert_eq!(o.page_levels(None).unwrap(), 2);
    assert_eq!(o.page_packed(None).unwrap()[..4], 2u32.to_le_bytes());
}

#[wasm_bindgen_test]
fn moves_drops_and_folding_levels() {
    let mut o = Outline::open("moves.sqlite", "memory").unwrap();
    let a = o.create(None, 0, "a").unwrap();
    let b = o.create(Some(a.clone()), 0, "b").unwrap();
    let c = o.create(None, 1, "c").unwrap();
    assert_eq!(value(&o.move_up(&c, None).unwrap())["structure_changed"], true);
    assert_eq!(texts(&mut o, None), ["c", "a", "b"]);
    o.move_down(&c, None).unwrap();
    let place = o.drop_place(Some(a.clone()), Some(c.clone()), Some(b.clone()), 0, None, 1).unwrap();
    assert_eq!(value(&place), serde_json::json!({ "parent": b, "index": 0, "depth": 1 }));
    o.drop_bullet(&c, &place).unwrap();
    assert_eq!(texts(&mut o, Some(a.clone())), ["b", "c"]);
    assert_eq!(o.drop_place(Some(a.clone()), Some(a.clone()), Some(b.clone()), 0, None, 1).unwrap(), "null");
    o.expand_to_level(None, 1).unwrap();
    assert_eq!(texts(&mut o, None), ["a"]);
    o.expand_to_level(None, u32::MAX).unwrap();
    o.move_to(&c, None).unwrap();
    assert_eq!(texts(&mut o, None), ["a", "b", "c"]);
    o.set_completed(&c, true).unwrap();
    o.set_hide_completed(true);
    assert!(o.hide_completed());
    assert_eq!(texts(&mut o, None), ["a", "b"]);
}

#[wasm_bindgen_test]
fn markup_formats_search_links_journal_and_trash() {
    let runs = value(&flowie_wasm::markup("**粗** #tag"));
    assert_eq!((runs[1]["start"].as_u64(), runs[1]["bold"].as_bool()), (Some(2), Some(true)));
    assert_eq!(runs.as_array().unwrap().last().unwrap()["kind"]["type"], "tag");
    let formatted = value(&flowie_wasm::toggle_format("a b", 2, 3, "bold").unwrap());
    assert_eq!(formatted, serde_json::json!({ "text": "a **b**", "start": 4, "end": 5 }));
    assert!(flowie_wasm::toggle_format("a", 0, 1, "loud").is_err());

    let mut o = Outline::open("lists.sqlite", "memory").unwrap();
    let a = o.create(None, 0, "**Errands**").unwrap();
    let milk = o.create(Some(a.clone()), 0, "buy milk #shop").unwrap();
    o.set_note(&milk, "oat").unwrap();
    assert_eq!(value(&o.search("oat", 10))[0]["id"], milk.as_str());
    assert_eq!(value(&o.quick_find("err", vec![], 10))[0]["text"], "Errands");
    assert_eq!(value(&o.bullet(&milk).unwrap())["path"][0]["text"], "Errands");
    assert_eq!(o.link(&a, "Untitled").unwrap(), format!("[Errands](flowie:{a})"));
    o.set_starred(&a, true).unwrap();
    let starred = value(&o.starred().unwrap());
    assert_eq!((starred[0]["text"].as_str(), starred[0]["plain"].as_str()), (Some("**Errands**"), Some("Errands")));
    assert_eq!(value(&o.index_pages())[0]["title"], "Errands");
    assert_eq!(value(&o.saved_searches()), serde_json::json!([]));
    o.create(None, 1, &format!("see [it](flowie:{a})")).unwrap();
    assert_eq!(value(&o.backlinks(&a).unwrap()).as_array().unwrap().len(), 1);

    let out = value(&o.open_day("2026-11-28").unwrap());
    assert_eq!(out["structure_changed"], true);
    let day = o.journal_day("2026-11-28").unwrap().unwrap();
    assert_eq!(o.journal_date(&day).as_deref(), Some("2026-11-28"));
    o.journal_append("2026-11-28", "first\nsecond").unwrap();
    assert_eq!(texts(&mut o, Some(day.clone())), ["first", "second"]);
    assert!(o.journal_day("tomorrow").is_err());

    o.delete(&milk).unwrap();
    let trash = value(&o.trashed());
    assert_eq!((trash[0]["id"].as_str(), trash[0]["count"].as_u64()), (Some(milk.as_str()), Some(1)));
    assert_eq!(value(&o.restore(&milk).unwrap())["focus"]["id"], milk.as_str());
    assert!(!o.is_empty());
}
