//! What a share-package import recorded, as plain data: the rows `store`
//! saves, and the before/after pair `History` keeps so undo can put them back.

use crate::tree::NodeId;

/// A root imported from a share package (one row per root). The bullet
/// shows an unread dot until `seen_at` is set.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportRecord {
    pub root: NodeId,
    pub package_id: String,
    pub title: String,
    pub author_id: String,
    pub author_name: String,
    pub exported_at: i64,
    pub imported_at: i64,
    pub seen_at: Option<i64>,
    pub source_ids: Vec<(NodeId, NodeId)>,
}

/// The merge bases of everything imported with `roots`.
#[derive(Clone, Debug, Default)]
pub struct ImportState {
    pub roots: Vec<NodeId>,
    pub records: Vec<ImportRecord>,
    pub bases: Vec<(NodeId, NodeId, String)>,
}
