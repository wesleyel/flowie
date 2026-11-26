//! Journaled changes, one field at a time.
//!
//! An edit is recorded as patches: a record created, a record removed, or one field set from
//! one value to another. Undo applies a field patch only while the field still holds the value
//! the patch set, so undoing never overwrites what something else changed since (another
//! device, through sync), and it leaves the other fields of the node alone. The same patches
//! tell what changed for saving and for refreshing the view (docs/roadmap/per-node-sync.md
//! §1 #6, P0.6).

use super::{BlockKind, Node, NodeId};
use crate::hlc::Reg;

/// The fields a patch can set. Position is one field (parent and rank together), so a move
/// is undone whole or not at all. Stars aren't here: they are not part of undo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Field {
    Position,
    Text,
    Note,
    Completed,
    Collapsed,
    Kind,
    Mirror,
    Image,
    Deleted,
    Modified,
}

impl Field {
    /// The synced register this field is part of (none for collapsed state and edit time).
    pub fn reg(self) -> Option<Reg> {
        match self {
            Field::Position => Some(Reg::Position),
            Field::Text => Some(Reg::Text),
            Field::Note => Some(Reg::Note),
            Field::Completed => Some(Reg::Completed),
            Field::Kind => Some(Reg::Kind),
            Field::Mirror => Some(Reg::Mirror),
            Field::Image => Some(Reg::Image),
            Field::Deleted => Some(Reg::Deleted),
            Field::Collapsed | Field::Modified => None,
        }
    }

    pub const ALL: [Field; 10] = [
        Field::Position,
        Field::Text,
        Field::Note,
        Field::Completed,
        Field::Collapsed,
        Field::Kind,
        Field::Mirror,
        Field::Image,
        Field::Deleted,
        Field::Modified,
    ];
}

/// A field's value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Position { parent: NodeId, rank: String },
    Text(String),
    Note(String),
    Completed(bool),
    Collapsed(bool),
    Kind(BlockKind),
    Mirror(NodeId),
    Image(String),
    Deleted(Option<i64>),
    Modified(i64),
}

impl Value {
    /// `node`'s value for `field`.
    pub fn of(node: &Node, field: Field) -> Value {
        match field {
            Field::Position => Value::Position { parent: node.parent, rank: node.rank.clone() },
            Field::Text => Value::Text(node.text.clone()),
            Field::Note => Value::Note(node.note.clone()),
            Field::Completed => Value::Completed(node.completed),
            Field::Collapsed => Value::Collapsed(node.collapsed),
            Field::Kind => Value::Kind(node.kind),
            Field::Mirror => Value::Mirror(node.mirror),
            Field::Image => Value::Image(node.image.clone()),
            Field::Deleted => Value::Deleted(node.deleted_at),
            Field::Modified => Value::Modified(node.modified),
        }
    }

    pub fn field(&self) -> Field {
        match self {
            Value::Position { .. } => Field::Position,
            Value::Text(_) => Field::Text,
            Value::Note(_) => Field::Note,
            Value::Completed(_) => Field::Completed,
            Value::Collapsed(_) => Field::Collapsed,
            Value::Kind(_) => Field::Kind,
            Value::Mirror(_) => Field::Mirror,
            Value::Image(_) => Field::Image,
            Value::Deleted(_) => Field::Deleted,
            Value::Modified(_) => Field::Modified,
        }
    }

    /// Write this value into `node`.
    pub fn apply(&self, node: &mut Node) {
        match self.clone() {
            Value::Position { parent, rank } => {
                node.parent = parent;
                node.rank = rank;
            }
            Value::Text(v) => node.text = v,
            Value::Note(v) => node.note = v,
            Value::Completed(v) => node.completed = v,
            Value::Collapsed(v) => node.collapsed = v,
            Value::Kind(v) => node.kind = v,
            Value::Mirror(v) => node.mirror = v,
            Value::Image(v) => node.image = v,
            Value::Deleted(v) => node.deleted_at = v,
            Value::Modified(v) => node.modified = v,
        }
    }

    /// The parent, for a position.
    pub fn parent(&self) -> Option<NodeId> {
        match self {
            Value::Position { parent, .. } => Some(*parent),
            _ => None,
        }
    }
}

/// The synced registers that differ between two states of a record (its star included,
/// which undo leaves out).
pub fn changed_regs(before: &Node, after: &Node) -> Vec<Reg> {
    let mut regs: Vec<Reg> =
        Field::ALL.into_iter().filter(|f| Value::of(before, *f) != Value::of(after, *f)).filter_map(Field::reg).collect();
    if before.star != after.star {
        regs.push(Reg::Star);
    }
    regs
}

/// What happened to a record.
#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    /// The record was created, as it was then.
    Create(Node),
    /// The record was removed for good, as it was then.
    Remove(Node),
    /// One field went from `before` to `after` (always the same field).
    Set { before: Value, after: Value },
}

/// One journaled change to one record.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub id: NodeId,
    pub edit: Edit,
}

impl Change {
    /// The patches that turn `before` into `after` (the same record), one per changed field.
    pub fn diff(before: &Node, after: &Node) -> Vec<Change> {
        Field::ALL
            .into_iter()
            .filter_map(|field| {
                let (b, a) = (Value::of(before, field), Value::of(after, field));
                (b != a).then_some(Change { id: after.id, edit: Edit::Set { before: b, after: a } })
            })
            .collect()
    }

    /// The field this sets, if it sets one.
    pub fn field(&self) -> Option<Field> {
        match &self.edit {
            Edit::Set { after, .. } => Some(after.field()),
            _ => None,
        }
    }

    /// Parents this change puts the record under or takes it from.
    pub fn parents(&self) -> Vec<NodeId> {
        match &self.edit {
            Edit::Create(n) | Edit::Remove(n) => vec![n.parent],
            Edit::Set { before, after } => [before.parent(), after.parent()].into_iter().flatten().collect(),
        }
    }
}
