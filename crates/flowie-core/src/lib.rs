//! Flowie core: the outline model, editing operations, undo, flattening,
//! search, inline markup and SQLite persistence. No UI code.
//!
//! Features (both on by default):
//! - `store`: SQLite persistence (`store`), share packages (`package`) and the image folder
//!   (`images::ImageDir`).
//! - `writer-thread`: `store::Writer`, the background thread that batches writes. Native only.
//!
//! With neither, what's left is the model (tree, undo, flatten, search, markup, text and OPML
//! import/export), which builds for `wasm32-unknown-unknown`.

pub mod flatten;
pub mod history;
pub mod hlc;
pub mod images;
pub mod imports;
pub mod io;
pub mod journal;
pub mod markup;
#[cfg(feature = "store")]
pub mod package;
pub mod rank;
pub mod search;
pub mod short_id;
#[cfg(feature = "store")]
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
