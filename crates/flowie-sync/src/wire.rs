//! What travels between devices (docs/roadmap/per-node-sync.md §4.1).
//!
//! Every backend moves [`Envelope`]s and never looks inside: the payload is JSON so that a
//! device on an older version keeps fields it doesn't know and writes them back unchanged.
//! Clocks travel as fixed-width hex text (`Hlc::to_wire`) and ids as 32 hex digits, which
//! stay exact in JavaScript; enums travel by name.

use flowie_core::hlc::Hlc;
use flowie_core::tree::{BlockKind, Node, NodeId, NodeSync, ROOT};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The envelope format this build writes.
pub const VERSION: u32 = 1;

/// What a record is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A bullet ([`NodeRecord`]).
    Node,
    /// The synced preferences (`store::PREFS`), one record, last writer wins.
    Prefs,
    /// Facts about the whole outline, such as the oldest version allowed to write it.
    Meta,
    /// A kind from a newer version: kept and passed on, never read.
    #[serde(other)]
    Unknown,
}

/// One record as a backend stores it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    /// The envelope format.
    pub v: u32,
    pub kind: Kind,
    /// The record's id: a node id for nodes.
    pub id: String,
    /// The backend's version of the record, from the last time it was saved or fetched; none
    /// for one it hasn't seen. Saving with a stale one is a conflict.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    /// Reserved for end-to-end encryption (`{alg, key_id, nonce}`); none means plain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enc: Option<Value>,
    pub payload: Value,
}

impl Envelope {
    pub fn node(record: &NodeRecord, etag: Option<String>) -> Envelope {
        Envelope {
            v: VERSION,
            kind: Kind::Node,
            id: record.id.to_string(),
            etag,
            enc: None,
            payload: serde_json::to_value(record).expect("a record is plain data"),
        }
    }

    /// The node record inside, if this is one this version can read.
    pub fn node_record(&self) -> Option<NodeRecord> {
        (self.kind == Kind::Node && self.enc.is_none()).then(|| serde_json::from_value(self.payload.clone()).ok()).flatten()
    }
}

/// A synced register: a value and the clock of its last write.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reg<T> {
    pub v: T,
    pub at: Hlc,
}

impl<T> Reg<T> {
    fn new(v: T, at: Hlc) -> Self {
        Reg { v, at }
    }
}

/// Where a node is: its parent and rank, and the parent it was moved from. One register,
/// so a move is atomic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub parent: NodeId,
    pub rank: String,
    pub from: NodeId,
}

/// A node as it syncs: each synced field with its clock (per-node-sync.md §3.2). A record
/// removed for good carries `removed`; it may still carry fields, written before or after.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeRecord {
    pub id: NodeId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<Hlc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Reg<Position>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<Reg<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<Reg<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed: Option<Reg<bool>>,
    /// By name (`BlockKind::name`); a name this version doesn't know reads as a bullet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<Reg<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mirror: Option<Reg<NodeId>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<Reg<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub star: Option<Reg<String>>,
    /// When it went to the Trash (milliseconds), or none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Reg<Option<i64>>>,
    /// Removed for good then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removed: Option<Hlc>,
    /// Fields a newer version wrote, kept as they are.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl NodeRecord {
    /// A record with no fields.
    pub fn empty(id: NodeId) -> Self {
        NodeRecord {
            id,
            created: None,
            position: None,
            text: None,
            note: None,
            completed: None,
            kind: None,
            mirror: None,
            image: None,
            star: None,
            deleted: None,
            removed: None,
            extra: Map::new(),
        }
    }

    /// A node removed for good at `at`.
    pub fn tombstone(id: NodeId, at: Hlc) -> Self {
        NodeRecord { removed: Some(at), ..NodeRecord::empty(id) }
    }

    /// A local node as it syncs.
    pub fn from_node(node: &Node) -> Self {
        use flowie_core::hlc::Reg as R;
        let c = &node.sync.clocks;
        let extra = match serde_json::from_str::<Value>(&node.sync.extra) {
            Ok(Value::Object(map)) => map,
            _ => Map::new(),
        };
        NodeRecord {
            id: node.id,
            created: Some(c.created),
            position: Some(Reg::new(
                Position { parent: node.parent, rank: node.rank.clone(), from: node.sync.moved_from },
                c.get(R::Position),
            )),
            text: Some(Reg::new(node.text.clone(), c.get(R::Text))),
            note: Some(Reg::new(node.note.clone(), c.get(R::Note))),
            completed: Some(Reg::new(node.completed, c.get(R::Completed))),
            kind: Some(Reg::new(node.kind.name().to_string(), c.get(R::Kind))),
            mirror: Some(Reg::new(node.mirror, c.get(R::Mirror))),
            image: Some(Reg::new(node.image.clone(), c.get(R::Image))),
            star: Some(Reg::new(node.star.clone(), c.get(R::Star))),
            deleted: Some(Reg::new(node.deleted_at, c.get(R::Deleted))),
            removed: None,
            extra,
        }
    }

    /// The latest clock of any field written, creation included.
    pub fn latest_write(&self) -> Option<Hlc> {
        [
            self.created,
            self.position.as_ref().map(|r| r.at),
            self.text.as_ref().map(|r| r.at),
            self.note.as_ref().map(|r| r.at),
            self.completed.as_ref().map(|r| r.at),
            self.kind.as_ref().map(|r| r.at),
            self.mirror.as_ref().map(|r| r.at),
            self.image.as_ref().map(|r| r.at),
            self.star.as_ref().map(|r| r.at),
            self.deleted.as_ref().map(|r| r.at),
        ]
        .into_iter()
        .flatten()
        .max()
    }

    /// Whether the node exists: it has fields, and was written after any removal. A device
    /// that wrote to a node after it was removed elsewhere (it hadn't seen the removal, or it
    /// undid it) brings it back rather than lose that write.
    pub fn exists(&self) -> bool {
        match (self.latest_write(), self.removed) {
            (None, _) => false,
            (Some(_), None) => true,
            (Some(written), Some(removed)) => written > removed,
        }
    }

    /// The node this record describes, if it exists. `local` gives what doesn't sync (its
    /// collapsed state), if this device has the node. A field missing from the record gets
    /// its default, written at the zero clock, so any real write wins over it.
    pub fn to_node(&self, local: Option<&Node>) -> Option<Node> {
        use flowie_core::hlc::Reg as R;
        if !self.exists() {
            return None;
        }
        let zero = Hlc::default();
        let created = self.created.unwrap_or(zero);
        let position = self.position.clone().unwrap_or(Reg::new(Position { parent: ROOT, rank: String::new(), from: ROOT }, zero));
        let mut node = Node::new(self.id, position.v.parent, position.v.rank);
        let mut sync = NodeSync { moved_from: position.v.from, ..NodeSync::default() };
        sync.clocks = flowie_core::hlc::Clocks::new(created);
        sync.clocks.set(R::Position, position.at);
        macro_rules! take {
            ($field:ident, $reg:expr, |$v:ident| $apply:expr) => {
                let at = match &self.$field {
                    Some(r) => {
                        let $v = r.v.clone();
                        $apply;
                        r.at
                    }
                    None => zero,
                };
                sync.clocks.set($reg, at);
            };
        }
        take!(text, R::Text, |v| node.text = v);
        take!(note, R::Note, |v| node.note = v);
        take!(completed, R::Completed, |v| node.completed = v);
        take!(kind, R::Kind, |v| node.kind = BlockKind::from_name(&v).unwrap_or_default());
        take!(mirror, R::Mirror, |v| node.mirror = v);
        take!(image, R::Image, |v| node.image = v);
        take!(star, R::Star, |v| node.star = v);
        take!(deleted, R::Deleted, |v| node.deleted_at = v);
        node.created = created.wall as i64;
        // The last edit of what the node shows: its content fields' latest clock.
        node.modified = [R::Text, R::Note, R::Completed, R::Kind, R::Image]
            .into_iter()
            .map(|r| sync.clocks.get(r).wall)
            .max()
            .unwrap_or(0)
            .max(created.wall) as i64;
        node.collapsed = local.is_some_and(|l| l.collapsed);
        if !self.extra.is_empty() {
            sync.extra = Value::Object(self.extra.clone()).to_string();
        }
        node.sync = sync;
        Some(node)
    }
}

/// The body of a save request to the self-hosted backend (`POST .../modify`).
#[derive(Debug, Serialize, Deserialize)]
pub struct ModifyRequest {
    pub saves: Vec<Envelope>,
}

/// Its answer: one result per save, in order.
#[derive(Debug, Serialize, Deserialize)]
pub struct ModifyResponse {
    pub results: Vec<crate::engine::SaveResult>,
}
