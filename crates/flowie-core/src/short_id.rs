//! Short node ids for display, like git's abbreviated hashes: the shortest
//! prefix of an id's hex digits that no other node shares, never fewer than
//! `MIN_LEN` digits. Anything stored (links, packages) uses the full id: a
//! prefix that is unique today may not be once more nodes exist.

use crate::tree::{NodeId, Tree, ROOT};

/// Fewest hex digits a short id has.
pub const MIN_LEN: usize = 6;
/// Fewest hex digits accepted when looking an id up by prefix.
pub const MIN_LOOKUP: usize = 4;

/// All node ids of a tree, sorted, at one revision of it.
#[derive(Debug, Default)]
pub struct ShortIds {
    revision: Option<u64>,
    sorted: Vec<NodeId>,
}

/// Why a prefix didn't name exactly one node.
#[derive(Debug, PartialEq, Eq)]
pub enum Lookup {
    /// Not hex, or shorter than `MIN_LOOKUP` digits.
    Invalid,
    NotFound,
    /// Several nodes start with it (the first few of them).
    Ambiguous(Vec<NodeId>),
}

/// Leading hex digits `a` and `b` have in common.
fn common_digits(a: NodeId, b: NodeId) -> usize {
    ((a.0 ^ b.0).leading_zeros() / 4) as usize
}

impl ShortIds {
    pub fn new(tree: &Tree) -> ShortIds {
        let mut s = ShortIds::default();
        s.update(tree);
        s
    }

    /// Re-read the tree's ids if it changed since the last call.
    pub fn update(&mut self, tree: &Tree) {
        if self.revision == Some(tree.revision()) {
            return;
        }
        self.revision = Some(tree.revision());
        self.sorted.clear();
        self.sorted.extend(tree.nodes().map(|n| n.id).filter(|id| *id != ROOT));
        self.sorted.sort_unstable();
    }

    /// Hex digits `id` needs to be told apart from every other node.
    pub fn len_of(&self, id: NodeId) -> usize {
        let Ok(i) = self.sorted.binary_search(&id) else { return 32 };
        let before = i.checked_sub(1).map_or(0, |j| common_digits(self.sorted[j], id));
        let after = self.sorted.get(i + 1).map_or(0, |&n| common_digits(n, id));
        (before.max(after) + 1).clamp(MIN_LEN, 32)
    }

    /// The short form of `id` (the full id if it isn't a node of the tree).
    pub fn short(&self, id: NodeId) -> String {
        let mut s = id.to_string();
        s.truncate(self.len_of(id));
        s
    }

    /// The node whose id starts with `prefix` (hex digits, any case, UUID
    /// hyphens allowed).
    pub fn resolve(&self, prefix: &str) -> Result<NodeId, Lookup> {
        let hex: String = prefix.chars().filter(|c| *c != '-').collect::<String>().to_ascii_lowercase();
        if hex.len() < MIN_LOOKUP || hex.len() > 32 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Lookup::Invalid);
        }
        let value = u128::from_str_radix(&hex, 16).map_err(|_| Lookup::Invalid)?;
        let free_bits = 4 * (32 - hex.len()) as u32;
        let lo = value.checked_shl(free_bits).unwrap_or(0);
        let hi = lo | if free_bits == 128 { u128::MAX } else { (1u128 << free_bits) - 1 };
        let start = self.sorted.partition_point(|id| id.0 < lo);
        let end = self.sorted.partition_point(|id| id.0 <= hi);
        match &self.sorted[start..end] {
            [] => Err(Lookup::NotFound),
            [one] => Ok(*one),
            many => Err(Lookup::Ambiguous(many.iter().take(5).copied().collect())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[u128]) -> ShortIds {
        let mut sorted: Vec<NodeId> = list.iter().map(|&n| NodeId(n)).collect();
        sorted.sort_unstable();
        ShortIds { revision: None, sorted }
    }

    const A: u128 = 0x3f9a1c00_0000_4000_8000_000000000001;
    const B: u128 = 0x3f9a1c70_0000_4000_8000_000000000002;
    const C: u128 = 0xb70e4200_0000_4000_8000_000000000003;

    #[test]
    fn six_digits_unless_a_neighbour_shares_more() {
        let s = ids(&[A, B, C]);
        assert_eq!(s.short(NodeId(C)), "b70e42");
        // A and B share 3f9a1c: they need a seventh digit.
        assert_eq!(s.short(NodeId(A)), "3f9a1c0");
        assert_eq!(s.short(NodeId(B)), "3f9a1c7");
        // Not a node: the full id.
        assert_eq!(s.len_of(NodeId(1)), 32);
    }

    #[test]
    fn prefixes_resolve_or_say_why_not() {
        let s = ids(&[A, B, C]);
        assert_eq!(s.resolve("b70e"), Ok(NodeId(C)));
        assert_eq!(s.resolve("B70E42"), Ok(NodeId(C)));
        assert_eq!(s.resolve("3f9a1c7"), Ok(NodeId(B)));
        assert_eq!(s.resolve(&NodeId(A).to_string()), Ok(NodeId(A)));
        assert_eq!(s.resolve(&uuid::Uuid::from_u128(A).hyphenated().to_string()), Ok(NodeId(A)));
        assert_eq!(s.resolve("3f9a1c"), Err(Lookup::Ambiguous(vec![NodeId(A), NodeId(B)])));
        assert_eq!(s.resolve("ffff"), Err(Lookup::NotFound));
        assert_eq!(s.resolve("b70"), Err(Lookup::Invalid));
        assert_eq!(s.resolve("zzzz"), Err(Lookup::Invalid));
    }

    #[test]
    fn follows_the_tree() {
        let mut t = Tree::new();
        let a = t.create_at(ROOT, 0, "a");
        let mut s = ShortIds::new(&t);
        assert_eq!(s.resolve(&s.short(a)), Ok(a));
        let b = t.create_at(ROOT, 1, "b");
        s.update(&t);
        assert_eq!(s.resolve(&s.short(b)), Ok(b));
        assert!(s.short(a).len() >= MIN_LEN);
    }
}
