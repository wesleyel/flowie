//! Flowie core: the outline model, editing operations, undo, flattening,
//! search, inline markup and SQLite persistence. No UI code.

pub mod flatten;
pub mod history;
pub mod io;
pub mod markup;
mod ops;
pub mod package;
pub mod search;
pub mod short_id;
pub mod store;
pub mod tree;

pub use flatten::{flatten, flatten_mapped, merge_leaving, DisplayRow, Row, ViewOptions};
pub use history::{Caret, History, Kind as HistoryKind};
pub use io::Item;
pub use search::Query;
pub use short_id::ShortIds;
pub use tree::{BlockKind, Node, NodeId, Tree, ROOT};

#[cfg(test)]
mod tests;
