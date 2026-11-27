//! Finding bullets: the palette's quick find (go to, move to, link to), search with the filter
//! syntax (`flowie_core::search`), and the Markdown link to a bullet. The desktop and the native
//! apps (flowie-ffi) both call these.

use flowie_core::search::Query;
use flowie_core::{markup, NodeId, Tree, ROOT};

/// Bullets whose text has every word of `query` (any case), best first: starred ones, then
/// ones starting with the first word, then short, shallow and recently changed ones. Mirrors,
/// empty bullets, and `exclude` with everything under them are left out. An empty query
/// matches every bullet with text. At most `limit`.
pub fn quick_find(tree: &Tree, query: &str, limit: usize, exclude: &[NodeId]) -> Vec<NodeId> {
    let words: Vec<String> = query.split_whitespace().map(|w| w.to_lowercase()).collect();
    let mut scored: Vec<(i64, NodeId)> = Vec::new();
    for n in tree.nodes() {
        if n.mirror != ROOT || exclude.iter().any(|e| *e == n.id || tree.is_ancestor(*e, n.id)) {
            continue;
        }
        if n.text.is_empty() {
            continue;
        }
        let text = n.text.to_lowercase();
        if !words.iter().all(|w| text.contains(w.as_str())) {
            continue;
        }
        let mut score = 0i64;
        if !n.star.is_empty() {
            score -= 1_000_000;
        }
        if let Some(first) = words.first() {
            if text.starts_with(first.as_str()) {
                score -= 10_000;
            }
        }
        score += n.text.len() as i64 + tree.ancestors(n.id).len() as i64 * 8;
        score -= (n.modified / 3_600_000) % 1000; // mild recency bias
        scored.push((score, n.id));
    }
    scored.sort_unstable();
    scored.into_iter().take(limit).map(|(_, id)| id).collect()
}

/// Every bullet `query` matches (mirrors left out), the most recently changed first.
pub fn search(tree: &Tree, query: &Query) -> Vec<NodeId> {
    let mut hits: Vec<NodeId> = tree.nodes().filter(|n| n.mirror == ROOT && query.matches(n)).map(|n| n.id).collect();
    hits.sort_by_key(|id| std::cmp::Reverse(tree.node(*id).modified));
    hits
}

/// A Markdown link to bullet `id` (a mirror's source): `[its text](flowie:ID)`, the text
/// plain, without brackets and cut to 48 characters; `untitled` when it has none.
pub fn link(tree: &Tree, id: NodeId, untitled: &str) -> String {
    let src = tree.source(id);
    let text = markup::plain_text(&tree.node(src).text);
    let text = if text.trim().is_empty() { untitled.to_string() } else { text };
    let label: String = text.replace(['[', ']'], "").chars().take(48).collect();
    format!("[{label}](flowie:{src})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_find_ranks_and_leaves_out() {
        let mut t = Tree::new();
        let a = t.create_at(ROOT, 0, "Groceries list");
        let b = t.create_at(a, 0, "groceries for the party");
        let c = t.create_at(ROOT, 1, "Party");
        t.create_at(ROOT, 2, "");
        t.star_at(c, 0);
        assert_eq!(quick_find(&t, "groceries", 10, &[]), [a, b], "starts with the word, then shallower");
        assert_eq!(quick_find(&t, "party", 10, &[]), [c, b], "starred first");
        assert_eq!(quick_find(&t, "groceries", 10, &[a]), Vec::<NodeId>::new(), "not into itself or under it");
        assert_eq!(quick_find(&t, "", 10, &[]).len(), 3, "an empty query: every bullet with text");
        assert_eq!(quick_find(&t, "", 1, &[]), [c]);
    }

    #[test]
    fn search_uses_the_filter_syntax() {
        let mut t = Tree::new();
        let a = t.create_at(ROOT, 0, "buy milk #errand");
        let b = t.create_at(ROOT, 1, "call mum #errand");
        t.create_at(ROOT, 2, "milk is in #fridge");
        t.set_completed(b, true);
        assert_eq!(search(&t, &Query::parse("#errand is:open")), [a]);
        let mut both = search(&t, &Query::parse("#errand"));
        both.sort();
        let mut want = vec![a, b];
        want.sort();
        assert_eq!(both, want);
        assert!(search(&t, &Query::parse("-milk -mum")).is_empty());
    }

    #[test]
    fn links_name_the_source() {
        let mut t = Tree::new();
        let a = t.create_at(ROOT, 0, "**Plan** [draft]");
        let empty = t.create_at(ROOT, 1, "");
        assert_eq!(link(&t, a, "Untitled"), format!("[Plan draft](flowie:{a})"));
        assert_eq!(link(&t, empty, "Untitled"), format!("[Untitled](flowie:{empty})"));
        let m = t.create_mirror(a, ROOT, 2).unwrap();
        assert_eq!(link(&t, m, "Untitled"), format!("[Plan draft](flowie:{a})"));
    }
}
