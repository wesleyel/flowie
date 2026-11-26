//! Per-node sync (docs/roadmap/per-node-sync.md): the records devices exchange, how two
//! versions of a record merge, and the engine that moves them. No IO and no threads: each
//! platform drives the engine from its own transport.

pub mod merge;
pub mod wire;

pub use merge::merge;
pub use wire::{Envelope, Kind, NodeRecord, Position, Reg, VERSION};
