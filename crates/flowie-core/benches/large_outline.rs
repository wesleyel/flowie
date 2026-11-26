//! Large-outline benchmark: time and live heap of the steps a big document goes through,
//! from import to Delete forever, plus the database file's size. It measures the
//! reports in https://github.com/wesleyel/flowie/discussions/7 (#16–#19).
//!
//! ```text
//! scripts/fetch-bench-data.sh                                 # optional: the real moby_dick.opml
//! cargo bench -p flowie-core --bench large_outline            # the novel ×5, as in the report
//! FLOWIE_BENCH_SCALE=20 cargo bench -p flowie-core --bench large_outline
//! ```
//!
//! The novel is `target/bench-data/moby_dick.opml` (or the OPML file in `FLOWIE_BENCH_OPML`)
//! when there is one, and otherwise a generated outline of about the same size.
//!
//! No benchmark framework: each step runs once on a fresh tree, timed with `Instant`, and a
//! counting global allocator reports the heap that stays live after it.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use flowie_core::{flatten, io, store, BlockKind, History, HistoryKind, Item, Tree, ViewOptions, ROOT};

struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            LIVE.fetch_add(new_size, Ordering::Relaxed);
            LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        }
        p
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

fn live() -> usize {
    LIVE.load(Ordering::Relaxed)
}

fn mb(bytes: usize) -> String {
    format!("{:.1} MB", bytes as f64 / 1e6)
}

/// Deterministic filler text: words from a small vocabulary, picked by a xorshift generator.
struct Words(u64);

impl Words {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn sentence(&mut self, words: usize) -> String {
        const VOCAB: [&str; 16] = [
            "whale", "sea", "ship", "captain", "harpoon", "voyage", "the", "and", "of", "deep", "white", "crew", "storm", "oil", "wind",
            "sail",
        ];
        (0..words).map(|_| VOCAB[(self.next() % 16) as usize]).collect::<Vec<_>>().join(" ")
    }
}

/// One novel-sized outline (about the size of Bike's MobyDick.opml): 135 chapters, each a
/// heading with paragraph rows under it.
fn novel(words: &mut Words) -> Item {
    let chapters = (1..=135)
        .map(|c| Item {
            text: format!("Chapter {c}"),
            kind: BlockKind::H2,
            children: (0..20).map(|_| Item { text: words.sentence(70), kind: BlockKind::Paragraph, ..Default::default() }).collect(),
            ..Default::default()
        })
        .collect();
    Item { text: "Moby-Dick".into(), kind: BlockKind::H1, children: chapters, ..Default::default() }
}

/// A Markdown article dump in the shape of the Wikipedia corpus from #19: headings of every
/// level, paragraphs separated by blank lines, and the odd list.
fn wiki_markdown(words: &mut Words, articles: usize) -> String {
    let mut md = String::new();
    for a in 0..articles {
        md.push_str(&format!("# Article {a}\n\n{}\n\n", words.sentence(60)));
        for s in 0..4 {
            md.push_str(&format!("## Section {s}\n\n{}\n\n{}\n\n", words.sentence(80), words.sentence(80)));
            md.push_str(&format!("### Part\n\n{}\n\n#### Detail\n\n{}\n\n", words.sentence(50), words.sentence(50)));
            md.push_str(&format!("- {}\n- {}\n\n", words.sentence(8), words.sentence(8)));
        }
    }
    md
}

struct Report(Vec<(String, Duration, String)>);

impl Report {
    fn step<T>(&mut self, name: &str, f: impl FnOnce() -> T) -> T {
        let start = Instant::now();
        let out = f();
        self.0.push((name.to_string(), start.elapsed(), mb(live())));
        out
    }
    fn note(&mut self, name: &str, value: String) {
        self.0.push((name.to_string(), Duration::ZERO, value));
    }
    fn print(&self) {
        println!("| step | time | live heap / value |\n|---|---:|---:|");
        for (name, time, value) in &self.0 {
            let time = if time.is_zero() { String::new() } else { format!("{:.1} ms", time.as_secs_f64() * 1e3) };
            println!("| {name} | {time} | {value} |");
        }
    }
}

fn main() {
    // `cargo test --benches` runs this with --bench too; keep that run small.
    let scale: usize = std::env::var("FLOWIE_BENCH_SCALE").ok().and_then(|s| s.parse().ok()).unwrap_or(5);
    let mut words = Words(0x9E37_79B9_7F4A_7C15);
    let mut r = Report(Vec::new());
    let base = live();
    r.note("baseline", mb(base));

    // Markdown import (#19).
    let md = wiki_markdown(&mut words, 200 * scale);
    r.note("wiki Markdown size", mb(md.len()));
    let parsed = r.step("parse wiki Markdown", || io::parse_any(&md));
    fn walk(items: &[Item], rows: &mut usize, note: &mut usize, hashes: &mut usize) {
        for it in items {
            *rows += 1;
            *note = (*note).max(it.note.len());
            *hashes += it.text.starts_with('#') as usize;
            walk(&it.children, rows, note, hashes);
        }
    }
    let (mut rows, mut longest_note, mut hashes) = (0, 0, 0);
    walk(&parsed, &mut rows, &mut longest_note, &mut hashes);
    r.note("  rows / longest note / rows starting with #", format!("{rows} / {longest_note} B / {hashes}"));
    drop(parsed);
    drop(md);

    // A large outline: import, undo step, expand, delete, delete forever (#16, #18).
    let opml = std::env::var_os("FLOWIE_BENCH_OPML")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/bench-data/moby_dick.opml"));
    let real = std::fs::read_to_string(&opml).ok().map(|xml| {
        let items = r.step("parse moby_dick.opml", || io::parse_opml(&xml));
        r.note("  OPML file", mb(xml.len()));
        items
    });
    let source = if real.is_some() { "moby_dick.opml" } else { "a generated novel" };
    let before_items = live();
    let items: Vec<Item> = (0..scale).flat_map(|_| real.clone().unwrap_or_else(|| vec![novel(&mut words)])).collect();
    drop(real);
    let mut tree = Tree::new();
    r.step(&format!("import {source} ×{scale} ({} rows)", Item::count(&items)), || tree.insert_items(ROOT, 0, &items));
    drop(items);
    let mut history = History::new();
    r.step("record the import as an undo step", || history.commit(&mut tree, HistoryKind::Structure, None, None));
    // Weigh the undo step by dropping the history; the steps below start without it.
    let with_undo = live() - before_items;
    drop(std::mem::take(&mut history));
    let tree_bytes = live() - before_items;
    r.note("  tree", mb(tree_bytes));
    r.note("  undo step for the import", mb(with_undo - tree_bytes));
    let shown = r.step("flatten, all expanded", || flatten(&tree, ROOT, &ViewOptions::default()));
    r.note("  visible rows", shown.len().to_string());
    drop(shown);

    // The database (#17).
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bench.sqlite");
    let mut conn = store::open(&path).unwrap();
    let dirty = tree.take_dirty();
    r.step("save to SQLite", || store::write_batch(&mut conn, &dirty, &[]).unwrap());
    drop(dirty);
    let size = |p: &std::path::Path| mb(std::fs::metadata(p).map(|m| m.len() as usize).unwrap_or(0));
    r.note("  database file", size(&path));

    let roots: Vec<_> = tree.children(ROOT).to_vec();
    r.step("delete every novel (to the Trash)", || {
        for id in &roots {
            tree.trash(*id);
        }
        history.commit(&mut tree, HistoryKind::Structure, None, None);
    });
    let dirty = tree.take_dirty();
    store::write_batch(&mut conn, &dirty, &[]).unwrap();
    drop(dirty);
    r.step("delete forever", || {
        for id in tree.trashed() {
            let ids = tree.purge(id);
            history.forget(&ids);
        }
    });
    let dirty = tree.take_dirty();
    r.step("  save the deletes", || store::write_batch(&mut conn, &dirty, &[]).unwrap());
    drop(dirty);
    r.note("  live heap left (tree + undo)", mb(live() - base));
    r.note("  database file", size(&path));
    r.step("  reclaim free pages", || store::reclaim_free_pages(&conn).unwrap());
    r.note("  database file", size(&path));
    drop(conn);
    drop(history);
    drop(tree);
    r.print();
}
