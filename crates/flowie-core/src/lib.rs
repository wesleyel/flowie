//! Flowie core: the outline model, editing operations, undo, flattening,
//! search, inline markup and SQLite persistence. No UI code.

pub mod flatten;
pub mod history;
pub mod markup;
mod ops;
pub mod search;
pub mod store;
pub mod tree;

pub use flatten::{flatten, merge_leaving, DisplayRow, Row, ViewOptions};
pub use history::{Caret, History, Kind as HistoryKind};
pub use search::Query;
pub use tree::{Node, NodeId, Tree, ROOT};

#[cfg(test)]
mod tests;
